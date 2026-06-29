# E7 — `lodestar-mcp` (fachada de agentes)

> **Fase**: `§14.7`. **Objetivo de la épica**: la 4ª fachada (rmcp, stdio) sobre la **misma**
> `Workspace`. Scope = **semántica, no CRUD** (Claude Code ya tiene Read/Write/Edit). El valor es lo
> que los ficheros crudos no dan barato: backlinks resueltos, ghosts, huérfanos, impacto, la puerta OKF,
> query estructurada y **escrituras validadas**. "Casi gratis" porque reusa la workspace.
> Referencias: `ARCHITECTURE.md §7.2`, `§13.7`, `§10` fila 14, `§12` (golden cross-fachada).

**Reglas duras**: logs **solo a stderr** (stdout = JSON-RPC); **no** expone `read_file`/`write_file`;
cada tool es un shell de 5–15 líneas sobre `Workspace`; `outputSchema` derivado vía feature `schemars` del core.

---

### E7-H01 — Servidor rmcp/stdio + puente del bus de eventos a tokio
- **Objetivo**: el servidor MCP base que abre el `Workspace` y puentea `IndexEvent` a tokio.
- **Referencias**: `ARCHITECTURE.md §7.2`, `§5` (el MCP puentea el bus a tokio).
- **Alcance**:
  - Servidor `rmcp` por stdio; **logs solo a stderr**, stdout reservado a JSON-RPC.
  - Abre `Workspace::open` (respeta el lockfile de "un bundle por proceso", `§12`).
  - Puentea `crossbeam IndexEvent` → tokio para **invalidar resources** en vivo.
- **Criterios de aceptación**:
  - El servidor arranca por stdio y responde al handshake MCP.
  - Ningún log contamina stdout (validado: stdout es JSON-RPC puro).
  - Un cambio de fichero invalida los resources afectados.
- **Dependencias**: E5-H01, E5-H03.
- **Pruebas**: handshake; stdout limpio; invalidación de resource tras edición.

### E7-H02 — Tools de lectura semántica
- **Objetivo**: exponer las tools de análisis que dan valor sobre el CRUD crudo.
- **Referencias**: `ARCHITECTURE.md §7.2`, `§4.2`, `§10` fila 14 (schemars).
- **Alcance**:
  - Tools: `find_backlinks` · `find_orphans` · `find_dangling` · `neighborhood(concept, depth, direction)` ·
    `conformance_check(path?)` · `query(dsl)`.
  - Cada tool = shell que llama **un** método de `Workspace` y serializa el DTO (camelCase del contrato).
  - **`outputSchema`** derivado de los DTO con la feature `schemars` del core (E1-H20).
- **Criterios de aceptación**:
  - Cada tool devuelve `structuredContent` con el DTO del contrato y un `outputSchema` válido.
  - `query` respeta la semántica de subcadena del core (no FTS-only).
- **Dependencias**: E7-H01, E1-H20, E5-H04.
- **Pruebas**: cada tool sobre fixture; validación de `outputSchema`.

### E7-H03 — Tools de escritura validada
- **Objetivo**: las escrituras del agente que rechazan no-conformidad con feedback accionable.
- **Referencias**: `ARCHITECTURE.md §7.2`, `§4.2` (rechazo = outcome, no Err), `§10` fila 13.
- **Alcance**:
  - `create_concept` (validado) · `update_frontmatter` (validado, **patch con null-borra**, RFC 7386) ·
    `generate_index` · `generate_tag_indexes`.
  - Pasan por el **único escritor** de la workspace; un rechazo devuelve el motivo + checks (no un error opaco).
- **Criterios de aceptación**:
  - `create_concept` no-conforme devuelve `written:false` + `rejected` + checks (el agente puede autocorregirse).
  - `update_frontmatter` con `clave:null` borra la clave.
  - Las escrituras nunca tocan la cache directamente (van por el único escritor).
- **Dependencias**: E7-H01, E5-H04.
- **Pruebas**: rechazo con feedback; null-borra; ruta de escritura única.

### E7-H04 — Resources read-only
- **Objetivo**: los resources de solo lectura que el agente puede listar/leer.
- **Referencias**: `ARCHITECTURE.md §7.2`.
- **Alcance**: resources: lista de concepts · índice de frontmatter · **gate de conformidad en vivo** · grafo de enlaces.
  Se **invalidan** con los `IndexEvent` (E7-H01).
- **Criterios de aceptación**: los 4 resources se listan y leen; el gate de conformidad refleja el estado actual.
- **Dependencias**: E7-H01, E7-H02.
- **Pruebas**: listar/leer cada resource; invalidación en vivo.

### E7-H05 — Tools de versionado para agentes (`history`/`diff`/`last_conforming`/`when_changed`/`commit`)
- **Objetivo**: dar al agente lectura de historial y la **única escritura git del MCP**: `commit`.
- **Referencias**: `ARCHITECTURE.md §13.7` (MCP), `§13.4`, `§12` (identidad: trailer Co-Authored-By).
- **Alcance**:
  - Lecturas: `history(concept?)` · `diff(revA, revB)` · `last_conforming_commit` · `when_changed(concept)`.
  - **`commit(message)`**: única escritura git del agente — hace **checkpoint** y **recibe la conformidad
    post-commit** (el agente aprende "no conforme" y se autocorrige).
  - Commits del agente con **trailer `Co-Authored-By`** distinguible (`git log`/blame no mienten, `§12`).
  - **push/pull y operaciones de rama quedan FUERA del MCP** (sync y topología = acciones humanas, `§13.7`).
- **Criterios de aceptación**:
  - `commit` devuelve la `CommitConformance` post-commit.
  - El commit del agente lleva el trailer `Co-Authored-By`.
  - No existen tools MCP de push/pull/branch (revisión).
- **Dependencias**: E7-H01, E5-H05, E4-H09.
- **Pruebas**: commit devuelve conformidad + trailer; ausencia de tools de red/rama.

### E7-H06 — Golden cross-fachada: CLI `--json` == MCP `structuredContent` == comando Tauri
- **Objetivo**: probar que las 3 fachadas devuelven exactamente el mismo DTO para la misma operación.
- **Referencias**: `ARCHITECTURE.md §12` (golden cross-fachada), `§10` fila 20 (golden de conformidad).
- **Alcance**:
  - Para un conjunto de operaciones (analyze/conformance/backlinks/query/neighborhood/diff/last_conforming):
    correr la CLI `--json`, el MCP `structuredContent` y el comando Tauri sobre la **misma** fixture y
    assertar **igualdad estructural**.
  - Incluir la conformidad por commit (golden cross-fachada del `§13.4`).
- **Criterios de aceptación**:
  - Las 3 salidas son idénticas (módulo orden, que el contrato fija con `BTreeMap`/`BTreeSet`).
  - Una divergencia hace fallar el job.
- **Dependencias**: E7-H02, E7-H05, E2-H02, E6-H01.
- **Pruebas**: el propio golden en CI.

### E7-H07 — Documentar el lanzamiento del MCP para Claude Code
- **Objetivo**: documentar el comando/registro del servidor MCP para Claude Code.
- **Referencias**: `ARCHITECTURE.md §12` (packaging: comando de lanzamiento del MCP documentado).
- **Alcance**: doc con el comando de arranque (stdio), el registro en la config de Claude Code, y la política de
  compat app/CLI/MCP/schema.
- **Criterios de aceptación**: un usuario puede registrar el MCP siguiendo la doc y las tools aparecen en Claude Code.
- **Dependencias**: E7-H01.
- **Pruebas**: smoke manual documentado; checklist de compat.
