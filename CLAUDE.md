# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

> Documento y UI en español (el usuario es hispanohablante). Mantén ese idioma en código,
> comentarios, mensajes de UI y commits salvo que se indique lo contrario.

## Estado actual del repo (importante)

Esto es **greenfield**. Hoy el repo contiene solo dos cosas:

- **`ARCHITECTURE.md`** (673 líneas) — el diseño **ratificado**. Es el contrato, no un borrador.
  Resume decisiones que **no son derivables del código** (porque el código casi no existe aún).
  **Léelo antes de tocar diseño o de escribir Rust/Svelte.**
- **`prototype/index.html`** (~2900 líneas, HTML/JS vanilla + localStorage) — el **prototipo de
  referencia**. Define el *comportamiento* esperado. La implementación real porta su lógica 1:1.

**Todavía NO existen** `Cargo.toml`, `package.json`, `crates/`, `src-tauri/`, ni infraestructura de
build/lint/test. No inventes comandos `cargo`/`npm` como si funcionaran: aún no hay nada que ejecuten.
La primera tarea de implementación es **scaffoldear el workspace** según `ARCHITECTURE.md §3`.

## Comandos

### Hoy (lo único que corre)
- **Ver el prototipo**: abrir `prototype/index.html` en un navegador (usa CDNs js-yaml/marked/jszip
  y `localStorage`; sin servidor ni build). Es la fuente de verdad del comportamiento a portar.

### Planificados (definidos en `ARCHITECTURE.md`, aún sin implementar)
Cuando exista el Cargo workspace, los comandos canónicos serán:
- `cargo test -p lodestar-core` — la lógica OKF se testea **sin GUI/DB/runtime** (core es puro).
- `lodestar check [--staged | --rev SHA | --range a..b]` — la **puerta de CI**. Exit codes
  congelados: `0` conforme · `1` hard-fail · `2` uso · `3` runtime/IO · `4` drift de generadores.
- Otros subcomandos CLI: `init` · `index` · `tags` · `export` · `reindex` · `import` + git
  (`log`/`diff`/`branch`/`merge`/`pull`/`push`/`hooks install`) (§7.3, §13.7).
- **Test de paridad** (obligatorio): SQL del store == `core::analyze` sobre la misma fixture (§5);
  test diferencial core-Rust vs prototipo-JS (§12).

## Arquitectura (el panorama)

`lodestar` = editor local-first de bases de conocimiento **OKF** (un directorio de `.md` con
frontmatter YAML). Stack ratificado: **Tauri v2 + Rust + Svelte 5/Vite + SQLite/FTS5 + git (libgit2 local +
binario git para red) + MCP (rmcp) + CLI (clap)**.

**Público objetivo: desarrolladores y perfiles técnicos.** Por eso git se expone con **vocabulario directo**
(commit/rama/push/pull), sin una capa de eufemismos para "ocultar complejidad". Decisión deliberada (§13).

### Grafo de crates (dirección de dependencia ◄ = "depende de")
```
lodestar-core   (PURO: modelo·conformidad·links·query·grafo·generación·export·diff. SIN I/O/DB/git/runtime)
   ▲        ▲
store      vcs  (store: rusqlite+FTS5+watcher notify, dueño del DDL .lodestar/index.db ·
   ▲        ▲    vcs: libgit2 local + binario git para red, dueño de git, NUNCA escribe el working tree)
   └─ workspace ─┘  (GLUE: compone core+store+vcs · handle unificado · ÚNICO escritor · bus de eventos)
        ▲  ▲  ▲
   src-tauri · lodestar-cli · lodestar-mcp   (3 fachadas finas: shells de 5–15 líneas, CERO lógica OKF)
```

### Invariantes no negociables (no relitigar sin motivo fuerte)
1. **Los `.md` en disco son la única fuente de verdad.** Todo lo demás se deriva y se reconstruye.
2. **`lodestar-core` es puro** — sin `tauri`/`rusqlite`/`notify`/`tokio`/`git2`. Solo modelo + lógica
   OKF. Lleva `#![forbid(unsafe_code)]`. (`rusqlite` vive solo en `store`; `git2` solo en `vcs`.)
3. **Una sola verdad computada**: backlinks, huérfanos, conformidad, query y grafo se computan con la
   misma lógica del core en las 3 fachadas. **SQLite es cache derivada/desechable**, verificada
   idéntica por el test de paridad; cuando podrían discrepar, **gana el core**.
4. **Un solo contrato de tipos**: `Check`/`Severity`/`CheckCode`/`Analysis`/`GraphModel`/… se definen
   **una vez** en `lodestar-core::types`. **Sin capa DTO paralela**; el `.d.ts` de TS se **genera**
   desde Rust (ts-rs/specta). (§4.1 fija los nombres/orden exactos — respétalos.)
5. **Un watcher = único escritor.** Los comandos **nunca** escriben la cache: escriben el `.md`
   (atómico temp+rename) y el watcher reconcilia (gate por hash blake3 que descarta echoes/no-ops).
6. **`RelPath` es un newtype validado** (rechaza absolutas/`..`) — único chokepoint de path-traversal.
   Prohibido `type RelPath = String`.
7. **git con vocabulario directo** (commit/rama/push/pull — público técnico, sin eufemismos). **Transporte
   híbrido**: libgit2 para lo local (no corre hooks al abrir/indexar = RCE-safe), binario `git` confinado a la
   red (push/pull/fetch). Scope v1: ramas locales (crear/cambiar/merge) + push/pull a remotos ya configurados;
   clone y gestión de remotos = no-goal.

### Flujo de datos (resumen)
Humano-en-app / agente-vía-MCP / git-pull → escriben un `.md` atómico → `notify` watcher (gate hash
blake3) → `store` upsert incremental a `.lodestar/index.db` + emite `IndexEvent` (crossbeam) →
`workspace` recomputa `Analysis` + snapshot → Tauri `app.emit('bundle:changed')` / MCP invalida
resources / CLI one-shot. (Diagrama completo en §9; integración git en §13.)

### Frontend (Svelte 5)
Porta la UI del prototipo **verbatim en aspecto** (mismo `<style>`, variables CSS, atributos
`data-*`) pero **invierte la propiedad de los datos**: el `files{}`/`analyzeBundle()` del prototipo se
van a Rust; la webview es vista fina sobre un `BundleSnapshot` empujado. **El grafo es una isla
imperativa** (`createStarMap`): posee el SVG y el loop rAF; Svelte le pasa nodos/aristas por métodos
en `$effect`, **nunca** con `{#each}` reactivo (§8).

## Cómo trabajar aquí

- **El prototipo es la spec de comportamiento.** Al portar lógica a Rust, busca la función original
  (`splitFront`, `parseFile`, `buildRaw`, `resolveLink`, `analyzeBundle`, `chk`, `tokenizeQuery`,
  `matchToken`, `confOf`, `diffSnap`/`fmDiff`/`lineDiff`/`collapseDiff`) y mantén su semántica —
  incluidos sus quirks (p. ej. el gating de fichero reservado **antes** de negar en query; `body:` y
  texto suelto son **subcadena**, no FTS). El arnés diferencial JS-vs-Rust es la red de seguridad.
- **Construir por fases** (`ARCHITECTURE.md §14`), validando con el arnés de paridad entre fases:
  1) core puro + contrato de tipos + diff OKF · 2) CLI mínima · 3) store (SQLite/FTS5 + watcher) ·
  4) vcs (libgit2 + git para red) + conformidad-por-commit · 5) workspace · 6) Tauri + frontend Svelte · 7) MCP ·
  8) transversales (migración, packaging, i18n, seguridad, config por-bundle, first-run).
- **`ARCHITECTURE.md` es la autoridad.** Las tablas §10 (decisiones ratificadas) y §12 (concerns
  transversales con dueño) resuelven contradicciones ya zanjadas; consúltalas antes de proponer un
  cambio de diseño. Si una decisión te parece equivocada, plantéalo explícitamente — no la deshagas
  por inercia.
