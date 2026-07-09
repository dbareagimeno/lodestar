# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

> Documento y UI en español (el usuario es hispanohablante). Mantén ese idioma en código,
> comentarios, mensajes de UI y commits salvo que se indique lo contrario.

## Estado actual del repo (importante)

**Todas las épicas (E0–E8) están implementadas y verificadas**: Cargo workspace de 7 crates +
`src-tauri`, frontend Svelte 5 funcional, CLI, MCP por stdio, store SQLite/FTS5 con watcher, vcs
git y workspace con bus en vivo. ~113 tests en verde, clippy `-D warnings` limpio. Lo pendiente es
**producto/pulido** (packaging/firma, rmcp, `.d.ts` generado), no arquitectura.

Mapa de documentos — quién manda sobre qué:

- **`ARCHITECTURE.md`** — el diseño **ratificado**; sigue siendo la autoridad sobre cualquier
  cuestión de diseño. Las tablas §10 (decisiones ratificadas) y §12 (concerns transversales con
  dueño) resuelven contradicciones ya zanjadas; consúltalas antes de proponer un cambio.
- **`IMPLEMENTATION_STATUS.md`** — estado real por épica/historia y qué invariantes están
  verificados. Actualízalo cuando cierres o abras trabajo.
- **`DECISIONES.md`** — decisiones abiertas que requieren criterio del usuario (rmcp, ts-rs,
  packaging, semántica de `--range`…). **No las tomes por tu cuenta**: propón y pregunta.
- **`prototype/index.html`** (~2900 líneas, HTML/JS vanilla + localStorage) — el **prototipo de
  referencia**: define el *comportamiento* que el core porta 1:1. Sigue siendo la spec de
  comportamiento; el arnés diferencial JS-vs-Rust (`prototype/harness/` +
  `crates/lodestar-core/tests/differential.rs`) lo ejecuta como **oráculo** en node.

## Comandos

### Build, test y lint (lo que corre el CI — `.github/workflows/ci.yml`)
```bash
cargo test --workspace --locked        # ~113 tests (incl. 6 diferenciales JS-vs-Rust)
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo doc --workspace --no-deps --locked   # con RUSTDOCFLAGS="-D warnings"
cd frontend && npm ci && npm run check && npm run build   # svelte-check + Vite → dist/
```
- El arnés diferencial necesita sus deps: `npm ci` en `prototype/harness/` (sin ellas, explota
  con `ERR_MODULE_NOT_FOUND` en vez de saltarse).
- El CI también verifica la **pureza del core** (`cargo tree -p lodestar-core` sin
  tokio/rusqlite/git2/notify/tauri) — no introduzcas esas deps en `lodestar-core`.

### CLI (`cargo run -p lodestar-cli -- …`)
- `check [--json|--sarif] [--staged | --rev SHA | --range a..b]` — la **puerta de CI**. Exit codes
  congelados: `0` conforme · `1` hard-fail · `2` uso · `3` runtime/IO · `4` drift de generadores.
  (`--range a..b` juzga la punta `b`.)
- `init [dir]` — bundle nuevo (index raíz, `.gitignore`, `git init` + commit inicial).
- `index [dir] [--check]` · `tags [--check]` — generadores (`--check` detecta drift → exit 4).
- `export [--out zip]` · `import [zip|dir]` — exporta / importa (zip del prototipo o directorio).
- `reindex` — reconstruye la cache `.lodestar/index.db`.
- git: `log` · `last-conforming` · `branch` · `switch [--create]` · `merge` · `pull` · `push` ·
  `hooks` (instala `pre-commit` → `lodestar check`).
- `--path <bundle>` es global; sin él sube desde el cwd buscando `index.md`/`.lodestar`.

### MCP y escritorio
```bash
cargo run -p lodestar-mcp -- <bundle>   # servidor MCP JSON-RPC por stdio (13 tools; stdout puro)
# Escritorio (Tauri v2). En debug la webview carga el dev server (devUrl :5173):
cd frontend && npm run dev              # terminal 1
cargo run -p lodestar-tauri             # terminal 2 → binario lodestar-desktop
# Sin dev server: cd frontend && npm run build && cargo run -p lodestar-tauri --release
```
En Linux, Tauri necesita libs de sistema (`libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev`, …); en macOS
no hace falta nada. `tauri.conf.json` embebe `frontend/dist` en el build (`generate_context!`).

## Arquitectura (el panorama)

`lodestar` = editor local-first de bases de conocimiento **OKF** (un directorio de `.md` con
frontmatter YAML). Stack ratificado: **Tauri v2 + Rust + Svelte 5/Vite + SQLite/FTS5 + git (libgit2 local +
binario git para red) + MCP + CLI (clap)**.

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
(`crates/lodestar-fixtures` provee los bundles de prueba compartidos por los tests.)

### Invariantes no negociables (no relitigar sin motivo fuerte)
1. **Los `.md` en disco son la única fuente de verdad.** Todo lo demás se deriva y se reconstruye.
2. **`lodestar-core` es puro** — sin `tauri`/`rusqlite`/`notify`/`tokio`/`git2`. Solo modelo + lógica
   OKF. Lleva `#![forbid(unsafe_code)]`. (`rusqlite` vive solo en `store`; `git2` solo en `vcs`.)
   El job `core-purity` del CI lo hace cumplir.
3. **Una sola verdad computada**: backlinks, huérfanos, conformidad, query y grafo se computan con la
   misma lógica del core en las 3 fachadas. **SQLite es cache derivada/desechable**, verificada
   idéntica por el test de paridad; cuando podrían discrepar, **gana el core**.
4. **Un solo contrato de tipos**: `Check`/`Severity`/`CheckCode`/`Analysis`/`GraphModel`/… se definen
   **una vez** en `lodestar-core::types`. **Sin capa DTO paralela**. (§4.1 fija los nombres/orden
   exactos — respétalos.) `frontend/src/lib/ipc/types.ts` es hoy un espejo **a mano**; generarlo con
   ts-rs/specta está pendiente (`DECISIONES.md §4`) — si tocas `core::types`, sincroniza el espejo.
5. **Un watcher = único escritor.** Los comandos **nunca** escriben la cache: escriben el `.md`
   (atómico temp+rename) y el watcher reconcilia (gate por hash blake3 que descarta echoes/no-ops).
6. **`RelPath` es un newtype validado** (rechaza absolutas/`..`) — único chokepoint de path-traversal
   (y de zip-slip en `import`). Prohibido `type RelPath = String`.
7. **git con vocabulario directo** (commit/rama/push/pull — público técnico, sin eufemismos). **Transporte
   híbrido**: libgit2 para lo local (no corre hooks al abrir/indexar = RCE-safe), binario `git` confinado a la
   red (push/pull/fetch). Scope v1: ramas locales (crear/cambiar/merge) + push/pull a remotos ya configurados;
   clone y gestión de remotos = no-goal. El `merge` es a nivel de árbol (libgit2 `merge_trees`): el vcs
   devuelve el `FileMap` y la workspace lo aplica por el único escritor.

### Flujo de datos (resumen)
Humano-en-app / agente-vía-MCP / git-pull → escriben un `.md` atómico → `notify` watcher (gate hash
blake3) → `store` upsert incremental a `.lodestar/index.db` + emite `IndexEvent` (crossbeam) →
`workspace` recomputa `Analysis` + snapshot → Tauri `app.emit('bundle:changed')` / MCP invalida
resources / CLI one-shot. (Diagrama completo en §9; integración git en §13.)

### Frontend (Svelte 5 — `frontend/`)
Porta la UI del prototipo **verbatim en aspecto** (mismo `<style>`, variables CSS, atributos
`data-*`) pero **invierte la propiedad de los datos**: el `files{}`/`analyzeBundle()` del prototipo
viven en Rust; la webview es vista fina sobre un `BundleSnapshot` empujado. **El grafo es una isla
imperativa** (`createStarMap` en `frontend/src/lib/graph/`): posee el SVG y el loop rAF; Svelte le
pasa nodos/aristas por métodos en `$effect`, **nunca** con `{#each}` reactivo (§8). i18n keyed por
`CheckCode` (`frontend/src/lib/i18n.ts`, catálogo español).

## Cómo trabajar aquí

- **El prototipo sigue siendo la spec de comportamiento.** Al tocar lógica del core, busca la
  función original (`splitFront`, `parseFile`, `buildRaw`, `resolveLink`, `analyzeBundle`, `chk`,
  `tokenizeQuery`, `matchToken`, `confOf`, `diffSnap`/`fmDiff`/`lineDiff`/`collapseDiff`) y mantén su
  semántica — incluidos sus quirks (p. ej. el gating de fichero reservado **antes** de negar en
  query; `body:` y texto suelto son **subcadena**, no FTS). **El arnés diferencial JS-vs-Rust es la
  red de seguridad**: ya cazó 6 divergencias reales (NFC en slugs, orden numérico de tags, `null` en
  YAML vacío, aristas a reservados, orden de extras).
- **Antes de mergear**: el CI exige fmt + clippy `-D warnings` + build `--all-targets` + tests +
  doc + pureza del core + frontend check/build. Ejecuta el subconjunto relevante en local.
- **`ARCHITECTURE.md` es la autoridad** en diseño; `DECISIONES.md` lista lo que está abierto a
  criterio del usuario — si una decisión ratificada te parece equivocada, plantéalo explícitamente,
  no la deshagas por inercia.
- **Mantén los documentos de estado**: si cierras algo de `DECISIONES.md` o cambias el estado de una
  épica, refleja el cambio en `IMPLEMENTATION_STATUS.md`/`DECISIONES.md` en el mismo PR.

## Flujo de trabajo con agentes (SDD · TDD · BDD · jueces ciegos)

El desarrollo se organiza con los agentes y skills de `.claude/` — mapa completo y workflows por
tipo de trabajo en [`.claude/README.md`](.claude/README.md):

| Skill | Cuándo |
|---|---|
| `/planificar <spec\|§N>` | Features grandes: diseño ratificado + épica de historias ordenadas (2 puertas). |
| `/historia <desc\|ID>` | Trabajo que cabe en una historia: spec en `requirements/` + ratificación. |
| `/tdd <ID>` | Rojo→verde→refactor con separación de poderes (autor-tests ≠ implementador). |
| `/juzgar [ID] [--panel]` | Juez **ciego** (agente fresco, solo spec+diff) antes de commitear/mergear. |
| `/contrato [--check]` | Coherencia de la frontera front↔back contra `contracts/*.yml`. |
| `/mutantes [--file ruta]` | cargo-mutants scoped: ¿la suite muerde donde tocaste? |
| `/ciclo <desc\|ID>` | Pipeline completo (historia→tdd→contrato→juez→docs→commit). Úsalo para features. |

Reglas de proceso: **nada se implementa sin historia ratificada**; el implementador **no puede
tocar los tests** del autor; a los jueces **nunca** se les pasa contexto de la conversación (esa es
la garantía de imparcialidad); `contracts/*.yml` describe la superficie de la frontera pero los
tipos viven solo en `core::types` (invariante #4).
