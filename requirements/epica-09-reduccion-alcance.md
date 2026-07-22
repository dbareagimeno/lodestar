# E9 — Reducción de alcance (giro headless)

> **Fase**: `§19.8` fase 0 (`REFACTOR §16`). **Objetivo de la épica**: ejecutar la **reducción de
> alcance** del giro a motor headless: retirar git de la **superficie de producto** (MCP + CLI)
> conservando el crate `lodestar-vcs` **dormido**, congelar la UI en el flujo de desarrollo, introducir
> la config nueva (`.lodestar/config.yaml` con `writableRoots`/`referenceRoots`/`ignored`) y la
> **separación canónico vs runtime**, y reposicionar la documentación de producto. Al cerrar E9 la
> superficie ya no expone git y el workspace distingue conocimiento, config y runtime.
> Referencias maestras: `ARCHITECTURE.md §19` (entero), `§19.1`, `§19.4`, `§13` (cabecera de
> supersesión), `§10` (nota del giro); `CLAUDE.md` invariantes #1/#5/#6.

**Principio rector de la épica**: *retirar exposición, no capacidad*. Git y su mecánica (`§13.2–§13.6`)
**se conservan** en el crate `vcs`; lo que se elimina es que alguna fachada lo **exponga**. Nada de esta
épica borra `lodestar-vcs` ni sus tests. Cualquier duda de "¿borro esto?" se resuelve: si es superficie
(tool/subcomando/campo de IPC) se retira; si es mecánica del crate, se deja dormida.

**Estado de la puerta 1**: `ARCHITECTURE.md §19` **ya está escrita** (ratificada 2026-07-22); esta épica
**no** la redacta, la **implementa**.

---

### E9-H01 — Retirar las tools git del MCP (`history`/`last_conforming_commit`/`commit`)
- **Objetivo**: la superficie MCP deja de exponer las 3 tools git; queda en 10 (o menos, hasta E10–E13).
- **Referencias**: `ARCHITECTURE.md §19.6`, `§19.1` · `REFACTOR §15` · `crates/lodestar-mcp/src/tools.rs:67-75`.
- **Alcance**:
  - Eliminar de `tools::list()` y del dispatcher las entradas `history`, `last_conforming_commit`, `commit`.
  - No tocar `Workspace::vcs_log`/`last_conforming`/`commit` (siguen existiendo, sin consumidor MCP).
  - Reescribir `contracts/mcp.yml` quitando esas 3 tools (el resto de la reescritura 13→10 es E10–E13).
- **Fuera de alcance**: añadir las tools nuevas (E10+); tocar la CLI (E9-H02).
- **Criterios de aceptación** (comportamiento):
  - **Dado** un servidor MCP arrancado, **Cuando** un cliente pide `tools/list`, **Entonces** no aparece
    ninguna de `history`/`last_conforming_commit`/`commit` → `list_sin_tools_git`.
  - **Dado** una petición `tools/call` con `name:"commit"`, **Cuando** se procesa, **Entonces** responde
    error de tool desconocida (`-32602`), no la ejecuta → `call_commit_desconocida`.
  - Estructural (checklist): `grep` en CI no encuentra `"history"`/`"commit"`/`"last_conforming_commit"`
    como nombres de tool en `tools.rs`; `contracts/mcp.yml` no las lista.
- **Dependencias**: —.
- **Pruebas**: `crates/lodestar-mcp/tests/`: `list_sin_tools_git`, `call_commit_desconocida`.
- **Frontera (mcp.yml)**: **sí**.

### E9-H02 — Retirar los subcomandos git de la CLI (conservando `check`)
- **Objetivo**: la CLI deja de exponer `log`/`last-conforming`/`branch`/`switch`/`merge`/`pull`/`push`/`hooks`.
- **Referencias**: `ARCHITECTURE.md §19.1`, `§19.6` · decisión **D-check** (`DECISIONES §0`) ·
  `crates/lodestar-cli/src/git.rs`, `crates/lodestar-cli/src/main.rs`.
- **Alcance**:
  - Quitar del enum de subcomandos de clap y del dispatch los 8 subcomandos git; borrar/vaciar el uso de
    `crates/lodestar-cli/src/git.rs` (el módulo puede quedar sin referenciar o eliminarse del árbol de la CLI).
  - **`check` permanece** como puerta de CI **sobre el working tree** (scope workspace).
  - `--staged`/`--rev`/`--range` de `check` quedan **diferidos** con el crate `vcs` dormido: se retiran de
    la superficie de `check` (documentar en el `--help` que el gate v2 juzga el working tree).
- **Fuera de alcance**: reescribir `check` como `knowledge_check` (eso es E14); tocar el MCP (E9-H01).
- **Criterios de aceptación**:
  - **Dado** `lodestar --help`, **Cuando** se imprime, **Entonces** no aparecen `log`/`branch`/`push`/…
    → `help_sin_subcomandos_git`.
  - **Dado** `lodestar check --rev HEAD`, **Cuando** se ejecuta, **Entonces** exit code `2` (uso: flag
    retirado), no juzga un árbol git → `check_rev_es_uso`.
  - **Dado** `lodestar check` en un bundle conforme, **Cuando** se ejecuta, **Entonces** exit `0` (la
    puerta sobre el working tree sigue viva) → `check_working_tree_conforme`.
  - Estructural: `grep` en CI no encuentra subcomandos git en el enum de clap de la CLI.
- **Dependencias**: —.
- **Pruebas**: `crates/lodestar-cli/tests/`: `help_sin_subcomandos_git`, `check_rev_es_uso`,
  `check_working_tree_conforme`.
- **Frontera (mcp.yml)**: no (CLI; `ipc.yml` no cambia).

### E9-H03 — Aislar `lodestar-vcs` como crate dormido (sin consumidores en fachadas)
- **Objetivo**: dejar `lodestar-vcs` compilando y con sus tests verdes, pero **sin ninguna fachada** que lo consuma.
- **Referencias**: `ARCHITECTURE.md §13` (cabecera de supersesión), `§19.2` · `crates/lodestar-vcs/`.
- **Alcance**:
  - Verificar (y documentar con un comentario de módulo) que `lodestar-mcp` y `lodestar-cli` **no**
    dependen ya de rutas de código que lleguen a `vcs` por la superficie; `Workspace` puede seguir
    exponiendo `vcs_*` internamente pero sin llamador de fachada.
  - Añadir a `ARCHITECTURE.md`/README la nota de "crate dormido" (ya en §13 cabecera; aquí solo el
    puntero desde el crate).
- **Fuera de alcance**: eliminar el crate o sus tipos de `core::types` (se conservan, decisión #1).
- **Criterios de aceptación**:
  - Estructural (checklist): `cargo tree -p lodestar-mcp` y `-p lodestar-cli` **no** listan una arista de
    uso de superficie hacia `vcs` (el crate puede seguir en el árbol vía `workspace`, pero ninguna tool/
    subcomando lo invoca); `cargo test -p lodestar-vcs` sigue verde.
  - **Dado** el workspace completo, **Cuando** se corre `cargo build --workspace`, **Entonces** compila
    sin warnings nuevos → (checklist de CI, no test unitario).
- **Dependencias**: E9-H01, E9-H02.
- **Pruebas**: se apoya en la suite existente de `lodestar-vcs` (sigue verde) + jobs de CI.
- **Frontera (mcp.yml)**: no.

### E9-H04 — Congelar la UI en el flujo de desarrollo (`.claude/`, `CLAUDE.md`, `docs/WORKFLOWS.md`)
- **Objetivo**: que el proceso de desarrollo trate `frontend/` y `src-tauri/` como **congelados** y no los toque.
- **Referencias**: `ARCHITECTURE.md §19.1` · decisión de UI congelada (`DECISIONES §0`) ·
  `.claude/README.md`, `.claude/skills/ux/`, `.claude/agents/disenador-ux.md`, `CLAUDE.md`, `docs/WORKFLOWS.md`.
- **Alcance**:
  - Anotar en `.claude/README.md` y `docs/WORKFLOWS.md` que el motor es **headless**: los skills `/ciclo`,
    `/historia`, `/ux` **no** modifican `frontend/`/`src-tauri/` en v2 (UI congelada); el skill `/ux` y el
    agente `disenador-ux` quedan marcados como no aplicables al giro headless.
  - Actualizar `CLAUDE.md`: sección de estado y mapa de crates para reflejar `lodestar-app`, el giro
    headless y la UI congelada (sin reescribir los invariantes, que siguen).
- **Fuera de alcance**: borrar la UI o sus tests; tocar el código de `frontend/`.
- **Criterios de aceptación**:
  - Estructural (checklist): `CLAUDE.md`, `.claude/README.md` y `docs/WORKFLOWS.md` mencionan
    explícitamente "UI congelada" y el crate `lodestar-app`; el mapa de crates incluye `lodestar-app`.
  - Revisión humana: un lector del flujo entiende que no debe tocar `frontend/`/`src-tauri/`.
- **Dependencias**: —.
- **Pruebas**: documentación (revisión); sin test automatizado.
- **Frontera (mcp.yml)**: no.

### E9-H05 — Config nueva `.lodestar/config.yaml` (`writableRoots`/`referenceRoots`/`ignored` + `gate` + `transactions`)
- **Objetivo**: el tipo `WorkspaceConfig` y su loader YAML, migrando lo útil de `lodestar.toml`.
- **Referencias**: `ARCHITECTURE.md §19.4` · `REFACTOR §4.2`, `§11.3` · decisión **D4/D5** ·
  `crates/lodestar-workspace/src/config.rs` (patrón `Config::load`).
- **Alcance**:
  - Tipo `WorkspaceConfig { workspace: { writable_roots: Vec<RelPath>, reference_roots: Vec<RelPath>,
    ignored: Vec<String> }, gate: { block_warnings: bool }, transactions: { retain_receipts_for, maximum_receipts } }`.
    `identity` se conserva como sección **dormida** (git fuera de superficie).
  - `WorkspaceConfig::load(root)` lee `.lodestar/config.yaml` (YAML) con defaults seguros si falta.
  - Defaults: `writableRoots = [ "." ]` (todo el bundle) si no se especifica; `referenceRoots = []`;
    `ignored` incluye siempre `.lodestar/runtime` y `.git`.
- **Fuera de alcance**: usar `referenceRoots` en validación (E11-H04); el modelo transaccional (E13).
- **Criterios de aceptación**:
  - **Dado** un `.lodestar/config.yaml` con `writableRoots: [knowledge]`, **Cuando** se carga, **Entonces**
    `writable_roots == [RelPath("knowledge")]` → `carga_writable_roots`.
  - **Dado** un bundle sin `config.yaml`, **Cuando** se carga, **Entonces** devuelve defaults seguros (no
    error) y `ignored` contiene `.lodestar/runtime` → `defaults_sin_config`.
  - **Dado** un `config.yaml` con `writableRoots: [../escape]`, **Cuando** se carga, **Entonces** error de
    validación (`RelPath` rechaza `..`) → `roots_rechazan_traversal`.
  - **Dado** un `config.yaml` malformado, **Cuando** se carga, **Entonces** error explícito (no defaults
    silenciosos) → `config_malformada_es_error`.
- **Dependencias**: —.
- **Pruebas**: `crates/lodestar-workspace/tests/`: `carga_writable_roots`, `defaults_sin_config`,
  `roots_rechazan_traversal`, `config_malformada_es_error`.
- **Frontera (mcp.yml)**: no.

### E9-H06 — Separación canónico vs runtime (`.lodestar/runtime/` + gitignore ajustado)
- **Objetivo**: el workspace distingue config versionada, conocimiento y runtime desechable.
- **Referencias**: `ARCHITECTURE.md §19.4` · `REFACTOR §4.1`, `§14` · decisión **D5** ·
  `crates/lodestar-workspace/src/lib.rs:96` (`ensure_cache_ignored`), `src/io.rs:9` (excludes del walker).
- **Alcance**:
  - Scaffold de `.lodestar/runtime/` (subdirs `plans/`, `receipts/`, `staging/`, y `audit.jsonl`, `journal`
    creados perezosamente) al abrir el workspace.
  - Ajustar el `.gitignore` generado/verificado para ignorar **solo** `.lodestar/index.db` +
    `.lodestar/runtime/` (ya **no** `.lodestar/` entero); `.lodestar/config.yaml`, `schema.yaml`,
    `templates/` quedan **versionados**.
  - El walker de `load_bundle` y el watcher ignoran `.lodestar/runtime/` y `index.db`, pero **incluyen** en
    la lectura de config los ficheros canónicos de `.lodestar/`.
- **Fuera de alcance**: usar staging/journal (E13); `WorkspaceRevision` (E10-H03, que ya excluye todo `.lodestar/`).
- **Criterios de aceptación**:
  - **Dado** un bundle recién abierto, **Cuando** se inspecciona el `.gitignore`, **Entonces** ignora
    `.lodestar/index.db` y `.lodestar/runtime/` pero **no** `.lodestar/config.yaml` → `gitignore_parte_lodestar`.
  - **Dado** un fichero en `.lodestar/runtime/plans/x.json`, **Cuando** el watcher procesa eventos,
    **Entonces** no genera un `IndexEvent` de conocimiento → `runtime_no_indexa`.
  - **Dado** un repo ya adoptado con `.lodestar/` trackeado entero, **Cuando** se abre, **Entonces** se
    ofrece/aplica ignorar solo `index.db`+`runtime/` (idempotente) → `adopcion_ajusta_gitignore`.
- **Dependencias**: E9-H05.
- **Pruebas**: `crates/lodestar-workspace/tests/`: `gitignore_parte_lodestar`, `runtime_no_indexa`,
  `adopcion_ajusta_gitignore`.
- **Frontera (mcp.yml)**: no.

### E9-H07 — Reposicionar la documentación de producto (README/CLAUDE/IMPLEMENTATION_STATUS)
- **Objetivo**: los documentos de estado reflejan el giro headless y el nuevo alcance.
- **Referencias**: `ARCHITECTURE.md §19.1` · `README.md`, `CLAUDE.md`, `IMPLEMENTATION_STATUS.md`.
- **Alcance**:
  - `README.md`/`CLAUDE.md`: posicionamiento como motor headless de integridad semántica; mapa de crates
    con `lodestar-app`; git fuera de superficie (crate dormido); UI congelada.
  - `IMPLEMENTATION_STATUS.md`: abrir el bloque de épicas E9–E14 (estado inicial "pendiente/en curso").
- **Fuera de alcance**: cambiar `ARCHITECTURE.md` (ya hecho en la puerta 1).
- **Criterios de aceptación**:
  - Estructural (checklist): `README.md` y `CLAUDE.md` describen el posicionamiento headless y citan
    `ARCHITECTURE.md §19`; `IMPLEMENTATION_STATUS.md` lista E9–E14.
- **Dependencias**: E9-H01…E9-H06 (para describir un estado real).
- **Pruebas**: documentación (revisión).
- **Frontera (mcp.yml)**: no.

---

## Orden de construcción (E9)

`E9-H01`, `E9-H02`, `E9-H04`, `E9-H05` son independientes (paralelizables). Luego `E9-H03` (necesita
H01+H02), `E9-H06` (necesita H05), y por último `E9-H07` (describe el estado ya alcanzado). Ninguna
historia está **[BLOQUEADA]**: todas las decisiones de diseño quedaron ratificadas en la puerta 1.
