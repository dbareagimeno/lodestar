# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

> Documento y UI en español (el usuario es hispanohablante). Mantén ese idioma en código,
> comentarios, mensajes de UI y commits salvo que se indique lo contrario.

## Estado actual del repo (importante)

**El repo COMPLETÓ el giro a motor headless de integridad semántica** (`ARCHITECTURE.md §19`,
ratificado 2026-07-22 — supersede `§13` en superficie de producto; épicas `E9`–`E14` en
`requirements/`). Lodestar deja de posicionarse como "editor local-first con git de primera clase"
y pasa a ser un **motor headless** consumido por agentes (MCP/CLI): **sin GUI y sin git en la
superficie**. El giro fue **aditivo, no destructivo** — retira exposición, no capacidad: nada se
borra, nada de lo ya construido deja de compilar.

Estado del giro — **E9–E14 COMPLETAS** (`IMPLEMENTATION_STATUS.md` tiene el detalle por historia):
- **Superficie MCP convergida a las 10 tools objetivo** (`§19.6`): `workspace_status`,
  `knowledge_search`, `knowledge_get`, `schema_inspect`, `graph_query`, `impact_analyze`,
  `knowledge_check`, `change_plan`, `change_apply`, `change_revert` (E14-H06 retiró las 10
  heredadas — `query`/`conformance_check`/`find_*`/`neighborhood`/`create_concept`/
  `update_frontmatter`/`generate_*` — a `contracts/mcp.yml §15`; ver `crates/lodestar-mcp/src/tools.rs`).
  Perfiles `readonly`/`standard` (`--profile`): readonly oculta Y rechaza las 3 tools de cambio.
- **git fuera de la superficie**: la CLI no expone subcomandos git ni `--staged`/`--rev`/`--range`
  en `check` — `check` sin flags juzga el **working tree** con conformidad completa (OKF + schema +
  refs) como puerta de CI (E14-H01). El crate `lodestar-vcs` **se conserva dormido** (compila, tests
  verdes; ninguna fachada lo invoca).
- **Modelo transaccional recuperable** (E12–E13): `change_plan` (normaliza/simula/valida, planHash) →
  `change_apply` (staging → lock → backup → write-ahead journal → renames atómicos → receipt, con
  crash-recovery determinista) → `change_revert`. El gate de staging valida la conformidad completa
  schema-driven (E14-H04, invariante #3). Auto-regen de `index`/`tags` dentro del apply (E13-H11).
- **Capa de servicios `lodestar-app`** introducida (envelope, códigos de error, casos de uso
  compartidos por MCP y CLI). **UI CONGELADA**: `frontend/`/`src-tauri/` no se tocan en el flujo de v2.
- Verificado end-to-end por el **benchmark funcional §17** (15 escenarios, E14-H04) y arnés de escala
  (~10k conceptos, E14-H05).

Herencia previa al giro — **las épicas E0–E8 están implementadas y verificadas**: Cargo workspace
de 7 crates + `src-tauri`, frontend Svelte 5 funcional (hoy congelado), CLI, MCP por stdio, store
SQLite/FTS5 con watcher, vcs git (hoy dormido) y workspace con bus en vivo. ~113 tests en verde,
clippy `-D warnings` limpio.

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
- `check [--json|--sarif]` — la **puerta de CI**. Exit codes congelados: `0` conforme · `1`
  hard-fail · `2` uso · `3` runtime/IO · `4` drift de generadores. Juzga siempre el **working
  tree** (scope workspace); desde `E9-H02` **no** expone `--staged`/`--rev`/`--range` (git fuera de
  la superficie de la CLI — quedan diferidos con el crate `vcs` dormido, ver `§19.1`).
- `init [dir]` — bundle nuevo (index raíz, `.gitignore`, `git init` + commit inicial).
- `index [dir] [--check]` · `tags [--check]` — generadores (`--check` detecta drift → exit 4).
- `export [--out zip]` · `import [zip|dir]` — exporta / importa (zip del prototipo o directorio).
- `reindex` — reconstruye la cache `.lodestar/index.db`.
- `--path <bundle>` es global; sin él sube desde el cwd buscando `index.md`/`.lodestar`.
- **Sin subcomandos git** (`log`/`last-conforming`/`branch`/`switch`/`merge`/`pull`/`push`/`hooks`
  retirados en `E9-H02`; la mecánica sigue en `lodestar-vcs`, dormida, por si vuelve a la
  superficie).

### MCP y escritorio (escritorio CONGELADO)
```bash
cargo run -p lodestar-mcp -- <bundle>   # servidor MCP JSON-RPC por stdio (10 tools, sin git; stdout puro)
# Escritorio (Tauri v2) — CONGELADO desde el giro headless (§19.1): se conserva compilando y
# funcional, pero el flujo de desarrollo (agentes/skills de `.claude/`) no lo toca en v2.
cd frontend && npm run dev              # terminal 1
cargo run -p lodestar-tauri             # terminal 2 → binario lodestar-desktop
# Sin dev server: cd frontend && npm run build && cargo run -p lodestar-tauri --release
```
En Linux, Tauri necesita libs de sistema (`libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev`, …); en macOS
no hace falta nada. `tauri.conf.json` embebe `frontend/dist` en el build (`generate_context!`).

## Arquitectura (el panorama)

`lodestar` es hoy un **motor headless de integridad semántica** (`ARCHITECTURE.md §19`) para bases
de conocimiento **OKF** (un directorio de `.md` con frontmatter YAML): sin GUI y sin git en la
superficie, consumido por agentes vía MCP/CLI. Stack: **Rust + SQLite/FTS5 + MCP + CLI (clap)**;
`Tauri v2 + Svelte 5/Vite` se conservan como la app de escritorio ya construida, hoy **congelada**
(no se toca en el flujo de desarrollo de v2). git sigue en el repo como capacidad **dormida**
(`lodestar-vcs`, libgit2 local + binario `git` para red) — la mecánica del §13 se conserva por si
la superficie vuelve a exponerlo, pero hoy ninguna fachada la invoca.

**Público objetivo: agentes (Claude Code, Codex, otros clientes MCP) y perfiles técnicos vía CLI.**
Cuando git estaba en superficie se exponía con **vocabulario directo** (commit/rama/push/pull), sin
eufemismos — esa decisión sigue documentada en `§13` para si vuelve.

### Grafo de crates (dirección de dependencia ◄ = "depende de")

Mapa del giro headless (`§19.2`; `lodestar-app` **ya existe** y las fachadas `lodestar-cli`/
`lodestar-mcp` lo consumen como capa de servicios de caso de uso):
```
lodestar-core   (PURO: modelo·conformidad·links·query·grafo·generación·export·diff. SIN I/O/DB/git/runtime)
   ▲        ▲
store      vcs  (store: rusqlite+FTS5+watcher notify, dueño del DDL .lodestar/index.db ·
   ▲        ▲    vcs: libgit2 local + binario git para red, DORMIDO — mecánica conservada,
   │        │    sin consumidor de fachada, NUNCA escribe el working tree)
   └─ workspace ─┘  (GLUE: compone core+store+vcs · handle unificado · ÚNICO escritor · bus de eventos)
        ▲
   lodestar-app   (servicios de caso de uso compartidos · envelope · códigos de error ·
                    CERO lógica de dominio; consumido por cli y mcp)
    ▲       ▲
lodestar-cli · lodestar-mcp   (fachadas finas: shells de 5–15 líneas, CERO lógica OKF; sin git)
src-tauri   (CONGELADO: sigue llamando a `workspace` directamente; UI congelada, no se toca)
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
   > **DORMIDO desde el giro headless** (`§19.1`): la mecánica de este invariante sigue viva en
   > `lodestar-vcs`, pero ninguna fachada (MCP/CLI) la expone hoy — se documenta como referencia por
   > si git vuelve a la superficie, no como comportamiento actual de producto.

### Flujo de datos (resumen)
Humano-en-app / agente-vía-MCP / git-pull → escriben un `.md` atómico → `notify` watcher (gate hash
blake3) → `store` upsert incremental a `.lodestar/index.db` + emite `IndexEvent` (crossbeam) →
`workspace` recomputa `Analysis` + snapshot → Tauri `app.emit('bundle:changed')` / MCP invalida
resources / CLI one-shot. (Diagrama completo en §9; integración git en §13.)

### Frontend (Svelte 5 — `frontend/`) — CONGELADO desde el giro headless

> **No se toca en el flujo de desarrollo de v2** (`§19.1`, `E9-H04`): ningún skill/agente de
> `.claude/` modifica `frontend/`/`src-tauri/` en el motor headless. Se documenta tal cual quedó
> construida (E0–E8), no como superficie activa.

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
tipo de trabajo en [`.claude/README.md`](.claude/README.md); guía explicativa (porqué del proceso,
pirámide, recetas con diagramas) en [`docs/WORKFLOWS.md`](docs/WORKFLOWS.md).

**El motor es headless (`§19.1`, `E9-H04`): `frontend/`/`src-tauri/` quedan CONGELADOS en el flujo
de desarrollo de v2.** `/ciclo`, `/historia`, `/tdd` no modifican esos directorios; el circuito UX
(`/ux` y el agente `disenador-ux`) queda **documentado pero no aplicable al giro headless** — se
conserva igual que git quedó dormido en `lodestar-vcs`, por si la UI vuelve a evolucionar.

| Skill | Cuándo |
|---|---|
| `/planificar <spec\|§N>` | Features grandes: diseño ratificado + épica de historias ordenadas (2 puertas). |
| `/historia <desc\|ID>` | Trabajo que cabe en una historia: spec en `requirements/` + ratificación. |
| `/tdd <ID>` | Rojo→verde→refactor con separación de poderes (autor-tests ≠ implementador). |
| `/juzgar [ID] [--panel]` | Juez **ciego** (agente fresco, solo spec+diff) antes de commitear/mergear. |
| `/contrato [--check]` | Coherencia de la frontera front↔back contra `contracts/*.yml`. |
| `/ux <flujo\|mockup\|audit>` | **No aplicable al giro headless** (UI congelada, `§19.1`) — documentado por si la UI vuelve a evolucionar; no se invoca en v2. |
| `/mutantes [--file ruta]` | cargo-mutants scoped: ¿la suite muerde donde tocaste? |
| `/ciclo <desc\|ID>` | Pipeline completo (historia→tdd→contrato→juez→docs→commit). Úsalo para features. **No toca `frontend/`/`src-tauri/`** en v2. |

Reglas de proceso: **nada se implementa sin historia ratificada**; el implementador **no puede
tocar los tests** del autor; a los jueces **nunca** se les pasa contexto de la conversación (esa es
la garantía de imparcialidad); `contracts/*.yml` describe la superficie de la frontera pero los
tipos viven solo en `core::types` (invariante #4).
