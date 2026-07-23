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
- **git fuera de la superficie y, desde `E15-H01`, fuera del repo**: la CLI no expone subcomandos
  git ni `--staged`/`--rev`/`--range` en `check` — `check` sin flags juzga el **working tree** con
  conformidad completa (OKF + schema + refs) como puerta de CI (E14-H01). El crate `lodestar-vcs`
  **se BORRÓ** en `E15-H01` (con `git2` y los tipos `Sha`/`Author`/`CommitRow`/…): ni `cargo tree`
  ni el `Workspace` saben ya de git.
- **Modelo transaccional recuperable** (E12–E13): `change_plan` (normaliza/simula/valida, planHash) →
  `change_apply` (staging → lock → backup → write-ahead journal → renames atómicos → receipt, con
  crash-recovery determinista) → `change_revert`. El gate de staging valida la conformidad completa
  schema-driven (E14-H04, invariante #3). La auto-regen de `index`/`tags` dentro del apply
  (E13-H11) **se retiró en `E15-H02`**: el apply publica exactamente lo que pide el change set.
- **Capa de servicios `lodestar-app`** introducida (envelope, códigos de error, casos de uso
  compartidos por MCP y CLI). **UI RETIRADA de `main`**: `frontend/` (Svelte) y `src-tauri/` se
  movieron íntegros a la rama `experimental/ui-desktop`; ya no forman parte del motor headless.
- Verificado end-to-end por el **benchmark funcional §17** (15 escenarios, E14-H04) y arnés de escala
  (~10k conceptos, E14-H05).

Herencia previa al giro — **las épicas E0–E8 están implementadas y verificadas**: Cargo workspace
de crates de core/store/workspace + fachadas, CLI, MCP por stdio, store SQLite/FTS5 con watcher y
workspace con bus en vivo (el crate `vcs`, construido en E4, se borró en `E15-H01`). La UI de
escritorio (`src-tauri` + frontend Svelte 5) se construyó y verificó en E0–E8, pero se **retiró de
`main` a la rama `experimental/ui-desktop`** con el giro headless (queda ahí íntegra como
referencia, no como parte del producto). Suite en verde, clippy `-D warnings` limpio.

Mapa de documentos — quién manda sobre qué:

- **`ARCHITECTURE.md`** — el diseño **ratificado**; sigue siendo la autoridad sobre cualquier
  cuestión de diseño. Las tablas §10 (decisiones ratificadas) y §12 (concerns transversales con
  dueño) resuelven contradicciones ya zanjadas; consúltalas antes de proponer un cambio.
- **`IMPLEMENTATION_STATUS.md`** — estado real por épica/historia y qué invariantes están
  verificados. Actualízalo cuando cierres o abras trabajo.
- **`DECISIONES.md`** — decisiones abiertas que requieren criterio del usuario (rmcp, ts-rs,
  packaging, semántica de `--range`…). **No las tomes por tu cuenta**: propón y pregunta.
- **`docs/REFACTOR_PHASE_2.md`** (+ `ARCHITECTURE.md §20`) — la **spec de comportamiento** vigente
  para la migración a workspace universal de Markdown.
- **`prototype/index.html`** (~2900 líneas, HTML/JS vanilla + localStorage) — **referencia
  histórica de v0.2.x**, ya NO la spec: `E15-H04` retiró su papel de oráculo junto con el arnés
  diferencial JS-vs-Rust (`crates/lodestar-core/tests/differential.rs`). Se conserva en el árbol
  para documentar el origen del modelo OKF; el CI ya no necesita node.

## Comandos

### Build, test y lint (lo que corre el CI — `.github/workflows/ci.yml`)
```bash
cargo test --workspace --locked        # ~113 tests (incl. 6 diferenciales JS-vs-Rust)
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo doc --workspace --no-deps --locked   # con RUSTDOCFLAGS="-D warnings"
```
- Sin node: `E15-H04` retiró el arnés diferencial, así que `cargo test` basta.
- El CI también verifica la **pureza del core** (`cargo tree -p lodestar-core` sin
  tokio/rusqlite/git2/notify/tauri/zip) y que el workspace entero no arrastre
  `git2`/`lodestar-vcs`/`zip` (retirados en `E15-H01`/`E15-H03`).

### CLI (`cargo run -p lodestar-cli -- …`)
- `check [--json|--sarif]` — la **puerta de CI**. Exit codes congelados: `0` conforme · `1`
  hard-fail · `2` uso · `3` runtime/IO. Juzga siempre el **working
  tree** (scope workspace); desde `E9-H02` **no** expone `--staged`/`--rev`/`--range`, y desde
  `E15-H01` no hay crate `vcs` que los soporte.
- `reindex` — reconstruye la cache `.lodestar/index.db`.
- **Retirados en E15**: `init` / `export` / `import` (`E15-H03`) e `index` / `tags` (`E15-H02`,
  con el exit code `4` de drift). La CLI queda en `check` + `reindex`.
- `--path <bundle>` es global; sin él sube desde el cwd buscando `index.md`/`.lodestar`.
- **Sin subcomandos git** (`log`/`last-conforming`/`branch`/`switch`/`merge`/`pull`/`push`/`hooks`
  retirados en `E9-H02`; la mecánica se borró en `E15-H01`, no queda dormida por si vuelve a la
  superficie).

### MCP
```bash
cargo run -p lodestar-mcp -- <bundle>   # servidor MCP JSON-RPC por stdio (10 tools, sin git; stdout puro)
```
La app de escritorio (Tauri v2 + Svelte 5) se **retiró de `main`** con el giro headless y vive en la
rama `experimental/ui-desktop`; sus comandos de desarrollo (`npm run dev`, `cargo run -p
lodestar-tauri`, libs de sistema Tauri) ya no aplican a este repo headless.

## Arquitectura (el panorama)

`lodestar` es hoy un **motor headless de integridad semántica** (`ARCHITECTURE.md §19`) para bases
de conocimiento **OKF** (un directorio de `.md` con frontmatter YAML): sin GUI y sin git en la
superficie, consumido por agentes vía MCP/CLI. Stack: **Rust + SQLite/FTS5 + MCP + CLI (clap)**. La
app de escritorio (`Tauri v2 + Svelte 5/Vite`) se **retiró de `main`** con el giro headless y se
conserva en la rama `experimental/ui-desktop`, ya fuera del producto. git **ya no está en el repo**:
`E15-H01` borró `lodestar-vcs` y `git2` (`ARCHITECTURE.md §20.13`); la mecánica del §13 solo existe
como documentación histórica.

**Público objetivo: agentes (Claude Code, Codex, otros clientes MCP) y perfiles técnicos vía CLI.**
Cuando git estaba en superficie se exponía con **vocabulario directo** (commit/rama/push/pull), sin
eufemismos — esa decisión sigue documentada en `§13` para si vuelve.

### Grafo de crates (dirección de dependencia ◄ = "depende de")

Mapa del giro headless (`§19.2`; `lodestar-app` **ya existe** y las fachadas `lodestar-cli`/
`lodestar-mcp` lo consumen como capa de servicios de caso de uso):
```
lodestar-core   (PURO: modelo·conformidad·links·query·grafo·plan·diff. SIN I/O/DB/git/runtime)
   ▲
store           (rusqlite+FTS5+watcher notify, dueño del DDL .lodestar/index.db)
   ▲
workspace       (GLUE: compone core+store · handle unificado · ÚNICO escritor · bus de eventos)
        ▲
   lodestar-app   (servicios de caso de uso compartidos · envelope · códigos de error ·
                    CERO lógica de dominio; consumido por cli y mcp)
    ▲       ▲
lodestar-cli · lodestar-mcp   (fachadas finas: shells de 5–15 líneas, CERO lógica OKF; sin git)
```
Las **dos únicas fachadas** son ahora `lodestar-cli` y `lodestar-mcp`; la fachada de escritorio
(`src-tauri`) se retiró de `main` a la rama `experimental/ui-desktop`.
(`crates/lodestar-fixtures` provee los bundles de prueba compartidos por los tests.)

### Invariantes no negociables (no relitigar sin motivo fuerte)
1. **Los `.md` en disco son la única fuente de verdad.** Todo lo demás se deriva y se reconstruye.
2. **`lodestar-core` es puro** — sin `tauri`/`rusqlite`/`notify`/`tokio`/`git2`/`zip`. Solo modelo +
   lógica OKF. Lleva `#![forbid(unsafe_code)]`. (`rusqlite` vive solo en `store`; `git2` ya no vive
   en ninguna parte: `E15-H01`.)
   El job `core-purity` del CI lo hace cumplir.
3. **Una sola verdad computada**: backlinks, huérfanos, conformidad, query y grafo se computan con la
   misma lógica del core en las 3 fachadas. **SQLite es cache derivada/desechable**, verificada
   idéntica por el test de paridad; cuando podrían discrepar, **gana el core**.
4. **Un solo contrato de tipos**: `Check`/`Severity`/`CheckCode`/`Analysis`/`GraphModel`/… se definen
   **una vez** en `lodestar-core::types`. **Sin capa DTO paralela**. (§4.1 fija los nombres/orden
   exactos — respétalos.) Los tipos los consumen directamente `lodestar-cli`/`lodestar-mcp`; **ya no
   hay espejo TS que sincronizar** — el `frontend/src/lib/ipc/types.ts` desapareció al retirar la UI
   a `experimental/ui-desktop`, y con él la nota de ts-rs/specta de `DECISIONES.md §4` (obsoleta para
   el espejo TS).
5. **Un watcher = único escritor.** Los comandos **nunca** escriben la cache: escriben el `.md`
   (atómico temp+rename) y el watcher reconcilia (gate por hash blake3 que descarta echoes/no-ops).
6. **`RelPath` es un newtype validado** (rechaza absolutas/`..`) — único chokepoint de
   path-traversal. Prohibido `type RelPath = String`.
7. ~~**git con vocabulario directo**~~ — **RETIRADO** (`ARCHITECTURE.md §20.13`, `E15-H01`). git
   salió de la superficie con el giro headless (`§19.1`) y del repo con E15: no hay `lodestar-vcs`,
   ni `git2`, ni tipos `Sha`/`CommitRow`/`Branch` en `core::types`. `§13` queda como histórico. Lo
   único que sobrevive es `workspace/src/gitignore.rs`, que gestiona el `.gitignore` del proyecto
   como **texto plano** para que la cache no se versione.

### Flujo de datos (resumen)
Agente-vía-MCP / CLI / edición externa → escriben un `.md` atómico → `notify` watcher (gate hash
blake3) → `store` upsert incremental a `.lodestar/index.db` + emite `IndexEvent` (crossbeam) →
`workspace` recomputa `Analysis` + snapshot → MCP invalida resources / CLI one-shot. (Diagrama
completo en §9; integración git en §13.)

### UI de escritorio — RETIRADA de `main`

La UI Svelte 5 (`frontend/`) y su fachada Tauri (`src-tauri/`) se **retiraron de `main`** con el giro
headless y viven íntegras en la rama `experimental/ui-desktop` — quien la quiera, esa rama. No forman
parte del motor headless ni del flujo de desarrollo de v2; su diseño se documenta en
`ARCHITECTURE.md §8`/`§13` como referencia histórica, no como superficie activa de este repo.

## Cómo trabajar aquí

- **La spec de comportamiento es `docs/REFACTOR_PHASE_2.md` + `ARCHITECTURE.md §20`**, no el
  prototipo. Desde `E15-H04` `prototype/index.html` es **referencia histórica de v0.2.x**: sirve
  para entender por qué el core hace lo que hace (los quirks portados 1:1 de `splitFront`,
  `parseFile`, `buildRaw`, `resolveLink`, `analyzeBundle`, `chk`, `tokenizeQuery`, `matchToken`,
  `confOf`, `diffSnap`/`fmDiff`/`lineDiff`/`collapseDiff` siguen ahí y siguen siendo el
  comportamiento actual), pero **ya no arbitra**: donde el prototipo y la spec de v0.3 discrepen,
  gana la spec. El arnés diferencial JS-vs-Rust se retiró con él; la red de seguridad de esa
  semántica son ahora los tests de `crates/lodestar-core/tests/core.rs`.
- **Antes de mergear**: el CI exige fmt + clippy `-D warnings` + build `--all-targets` + tests +
  doc + pureza del core. Ejecuta el subconjunto relevante en local.
- **`ARCHITECTURE.md` es la autoridad** en diseño; `DECISIONES.md` lista lo que está abierto a
  criterio del usuario — si una decisión ratificada te parece equivocada, plantéalo explícitamente,
  no la deshagas por inercia.
- **Mantén los documentos de estado**: si cierras algo de `DECISIONES.md` o cambias el estado de una
  épica, refleja el cambio en `IMPLEMENTATION_STATUS.md`/`DECISIONES.md` en el mismo PR.

## Flujo de trabajo con agentes (SDD · TDD · BDD · jueces ciegos)

El desarrollo se organiza con los agentes y skills de `.claude/` — mapa completo y workflows por
tipo de trabajo en [`.claude/README.md`](.claude/README.md); guía explicativa (porqué del proceso,
pirámide, recetas con diagramas) en [`docs/WORKFLOWS.md`](docs/WORKFLOWS.md).

**El motor es headless (`§19.1`, `E9-H04`): la UI de escritorio se retiró de `main` a la rama
`experimental/ui-desktop`.** Con ella se retiraron el skill `/ux` y el agente `disenador-ux` (el
circuito UX ya no existe en `main`); si la UI vuelve a evolucionar, se hace en esa rama.

| Skill | Cuándo |
|---|---|
| `/planificar <spec\|§N>` | Features grandes: diseño ratificado + épica de historias ordenadas (2 puertas). |
| `/historia <desc\|ID>` | Trabajo que cabe en una historia: spec en `requirements/` + ratificación. |
| `/tdd <ID>` | Rojo→verde→refactor con separación de poderes (autor-tests ≠ implementador). |
| `/juzgar [ID] [--panel]` | Juez **ciego** (agente fresco, solo spec+diff) antes de commitear/mergear. |
| `/contrato [--check]` | Coherencia de la frontera MCP↔`core::types` contra `contracts/mcp.yml`. |
| `/mutantes [--file ruta]` | cargo-mutants scoped: ¿la suite muerde donde tocaste? |
| `/ciclo <desc\|ID>` | Pipeline completo (historia→tdd→contrato→juez→docs→commit). Úsalo para features. |

Reglas de proceso: **nada se implementa sin historia ratificada**; el implementador **no puede
tocar los tests** del autor; a los jueces **nunca** se les pasa contexto de la conversación (esa es
la garantía de imparcialidad); `contracts/*.yml` describe la superficie de la frontera pero los
tipos viven solo en `core::types` (invariante #4).
