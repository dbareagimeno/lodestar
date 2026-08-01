# Propuesta de diseño — Lodestar como motor headless de integridad semántica

> **Fase A de `/planificar`** para el giro de `docs/REFACTOR.md`. Esto es una **propuesta**: no
> está ratificada. No modifica `ARCHITECTURE.md` ni `DECISIONES.md`. Tras la ratificación del
> usuario se escribirá la **adenda** en `ARCHITECTURE.md` (que supersede §13 parcialmente) y se
> pasará a la Fase B (descomposición en épica de historias).
>
> Fuente de la spec: `docs/REFACTOR.md` (secciones §1–§18). Autoridad de diseño actual:
> `ARCHITECTURE.md` (§10 decisiones ratificadas, §12 concerns, §13 git). Invariantes: `CLAUDE.md` #1–#6.

## 0. Resumen de posicionamiento

Lodestar deja de ser un "editor local-first con git de primera clase" (posición de `ARCHITECTURE.md`
§1/§13) y pasa a ser un **motor headless de integridad semántica** que agentes MCP y la CLI usan para
buscar, comprender, validar y **modificar conocimiento mediante cambios planificados y recuperables**,
sin poseer el editor, git ni el entorno de desarrollo (`REFACTOR.md §1, §18`).

Los **invariantes #1–#6 de `CLAUDE.md` siguen íntegros** y de hecho encajan mejor con el nuevo
posicionamiento (los `.md` como única verdad, core puro, una verdad computada, un contrato de tipos,
un único escritor, `RelPath` como chokepoint). El giro **no relitiga** ninguno; los usa como cimiento.

Decisiones del usuario ya **cerradas** (incorporadas, no se discuten):

1. **Git sale de la superficie de producto**: fuera las tools MCP `history`/`last_conforming_commit`/
   `commit` y los subcomandos CLI de `crates/lodestar-cli/src/git.rs`
   (`log`/`last-conforming`/`branch`/`switch`/`merge`/`pull`/`push`/`hooks`). **El crate
   `lodestar-vcs` NO se elimina**: queda aislado, sin consumidores en las fachadas (por si vuelve).
   Los tipos git de `core::types` (`Sha`, `CommitRow`, `Branch`, `OkfDiff`…) pueden quedarse aunque
   dejen de exponerse.
2. **UI congelada**: no se toca `frontend/` ni `src-tauri/`. Se trata la app como motor headless
   aunque la UI exista (con zonas sin backend nuevo). El flujo de desarrollo (`.claude/`, `CLAUDE.md`,
   `docs/WORKFLOWS.md`) se actualiza para tratar la UI como congelada — es trabajo de la épica Fase 0.
3. **Ejecución por fases** con puertas de ratificación (proceso, no diseño).

---

## a) Posicionamiento y reversión controlada de §13

**Propuesta de redacción de la adenda** (a escribir en `ARCHITECTURE.md` tras ratificar):

- Se añade una sección nueva (p. ej. **§19 "Motor headless de integridad semántica (supersede §13
  en superficie)"**) que:
  - Declara el nuevo posicionamiento (§0 de este documento).
  - **Supersede §13 SOLO en cuanto a *exponer* git por MCP/CLI.** §13 **no se borra**: se le antepone
    una nota de cabecera *"Superada por §19 en cuanto a superficie de producto: el crate `lodestar-vcs`
    y su mecánica interna (§13.2–§13.6) se conservan, pero ninguna fachada los expone en v2."* El
    diseño interno de git (transporte híbrido, conformidad por commit, único escritor, `OKF-CONFLICT`,
    checkpoint) queda **documentado como dormido**, no como implementación viva de producto.
  - Marca en la tabla **§10** una fila nueva o una anotación: la decisión #15 (dónde vive git) y #16–#21
    (git de primera clase) siguen siendo **ciertas sobre el crate**, pero su *exposición* queda revertida.
    No se editan las filas #1–#14 (siguen vigentes tal cual).
- **Lo que NO cambia de §13**: el crate `lodestar-vcs` compila, sus tests siguen en verde, y
  `Workspace` puede seguir teniendo los métodos `vcs_*` internamente; simplemente **ninguna fachada los
  llama**. El guardián de contrato vigilará que `contracts/mcp.yml`/`ipc.yml` no reintroduzcan git.

> **Punto para el usuario (D0 — confirmación de forma):** ¿la adenda es una **§19 nueva** que antepone
> una nota a §13 (recomendado: preserva la historia de diseño y deja git "en el congelador"), o
> prefieres **reescribir §13** in situ marcándola como superada? Recomiendo §19 nueva + nota en §13.

---

## b) Dónde viven las capas nuevas (el punto de diseño central)

### b.1 ¿Crate nuevo `lodestar-app` o ampliar `lodestar-workspace`?

Hoy MCP y CLI llaman **directamente** a `lodestar_workspace::Workspace`
(`crates/lodestar-mcp/src/tools.rs:9`, `crates/lodestar-cli/src/git.rs:6`). `Workspace`
(`crates/lodestar-workspace/src/lib.rs:34`) es el glue: **único escritor** (`io.rs:63 write_atomic`),
dueño de la cache/watcher/bus y de la config (`config.rs`).

`REFACTOR.md §3` exige: *"MCP y CLI no deben contener lógica de dominio; ambos deben invocar los mismos
servicios de aplicación."* La orquestación nueva (change_plan que ensambla un `ChangeSet` con
riesgo+diff+validación+plan persistido; change_apply con el proceso de 15 pasos §11.2; expiry de planes;
receipts; envelope; mapa de códigos de error) es **sustancial y con estado**, y no es lo mismo que la
mecánica de escritura de bajo nivel.

| Opción | Qué implica | Trade-offs |
|---|---|---|
| **A — Crate nuevo `lodestar-app`** que orquesta todo (core+store+workspace) y es el ÚNICO consumidor de las fachadas | mcp/cli pasan a depender solo de `lodestar-app`; el modelo transaccional entero vive ahí | Separación limpia y testeable; pero duplica responsabilidad con workspace y **puede tensionar el invariante #5** si el staging/escritura viven fuera del "único escritor" |
| **B — Ampliar `lodestar-workspace`** con los casos de uso como métodos nuevos | change_plan/apply/revert son métodos de `Workspace` | Cero crates nuevos; pero `Workspace` (ya grande) mezcla IO de bajo nivel + orquestación de alto nivel + envelope; peor aislamiento de test |
| **C — Híbrido (recomendado)** | La **mecánica transaccional** (staging, journal, locks, aplicación atómica por lotes, crash-recovery) vive en `lodestar-workspace`, **junto al único escritor y a `.lodestar/runtime/`**. Se introduce **`lodestar-app`** como capa **fina** de casos de uso que ambas fachadas consumen: ensambla `ChangeSet`, conduce plan→validar→aplicar→verificar, construye el **envelope**, mapea `CoreError`/`WorkspaceError`→códigos de error estables | Honra el invariante #5 (el escritor sigue en workspace) y §3 de REFACTOR (mcp/cli sobre los mismos servicios, sin dominio en las fachadas). Coste: un crate más y una frontera más que mantener |

**Recomendación: Opción C.** El invariante #5 obliga a que la escritura transaccional (renames atómicos
por lote, journal, recovery) siga siendo del único escritor en `lodestar-workspace`; pero la lógica de
*caso de uso* (que es lo que REFACTOR llama "servicios de aplicación") gana un hogar propio en
`lodestar-app`, delgado, sin `rusqlite`/`git2`/`tokio`, que compone `core` (validación/diff/schema puros)
+ `workspace` (escritura/cache/locks). Grafo de dependencia resultante:

```
core (PURO)  ◄─ store ─┐   core ◄─ vcs (DORMIDO, sin consumidores)
   ▲                   ▼
   └──────── workspace (único escritor + staging/journal/locks/recovery + cache)
                   ▲
             lodestar-app   (servicios de aplicación / casos de uso · envelope · códigos de error)
                   ▲   ▲
              lodestar-mcp · lodestar-cli     (fachadas finas: 5–15 líneas, CERO dominio)
              src-tauri (CONGELADO: sigue llamando a workspace directamente; no se toca)
```

> **Punto para el usuario (D1):** ¿Opción **C** (mecánica en workspace + `lodestar-app` fino, recomendado),
> **A** (todo en un `lodestar-app` grande) o **B** (ampliar workspace, cero crates nuevos)?

### b.2 El subsistema de schemas → en `lodestar-core` (PURO). **Confirmado.**

`DocType`, `requiredFields`, `allowedStatuses`, typed relations, lifecycle y templates
(`REFACTOR.md §4, §9.4`) son **lógica de validación de dominio sin I/O** → pertenecen a `lodestar-core`,
que es puro (invariante #2). Concretamente:

- **En core** (nuevo módulo `core::schema`): el tipo `Schema` (catálogo de `DocType`, campos, relaciones,
  reglas, lifecycle, plantillas) y las **funciones de validación** que, dado un `Schema` + un `Bundle`,
  producen `Vec<Check>` (extiende el `conform` actual de 15 checks; ver la tensión §c/§Tensiones sobre
  `CheckCode`). La **aplicación de plantillas** (generar frontmatter+cuerpo desde un `DocType`) es
  generación pura, como `gen_index`/`gen_tag_indexes` (`bundle.rs:207`).
- **En workspace** (I/O): **leer** `.lodestar/schema.yaml` y `.lodestar/templates/` de disco y
  deserializarlos a `Schema` — exactamente el patrón de `Config::load` (`config.rs:41`), que hoy lee
  `lodestar.toml` y entrega datos al core. El core **nunca** abre ficheros.

Esto respeta el invariante #2 (core sin I/O) y #3 (la validación de schema es "una verdad computada" del
core, no de SQLite). **No requiere criterio del usuario**; se declara en la adenda.

### b.3 El modelo transaccional → mecánica en `lodestar-workspace`, orquestación en `lodestar-app`.

`REFACTOR.md §5` (staging, write-ahead journal, locks de workspace, copias de recuperación, crash
recovery, receipts) toca el disco y el orden de escritura → **es del único escritor** (invariante #5) y
vive en `lodestar-workspace`, no en el core ni en una capa paralela. Anclajes:

- **Único escritor**: hoy `write_atomic` (`io.rs:63`) es por-fichero (temp+fsync+rename). Una transacción
  multi-fichero es un **lote** por el mismo camino: se materializa el resultado en staging, se valida, se
  toma el lock, se re-verifica la revisión base, se copian los originales a recuperación, y se publican
  los renames uno a uno registrándolos en el journal. Sigue siendo **un solo escritor**; el lote no
  introduce un segundo.
- **Invariante #1 respetado**: staging, journal, receipts, planes y `audit.jsonl` viven en
  **`.lodestar/runtime/`** (`REFACTOR.md §4.1, §11.1, §14`) — **derivado/desechable, NO canónico**. Los
  `.md` de `writableRoots` siguen siendo la única fuente de verdad. `.lodestar/runtime/` no entra en el
  índice, ni en `WorkspaceRevision`, ni en las validaciones (§d).
- **Crash recovery**: al abrir, `Workspace::open_live` detecta un journal incompleto y ejecuta la
  estrategia determinista (completar o restaurar) **antes** de servir lecturas —
  `workspace_status.recovery.pendingTransaction` lo expone (`REFACTOR.md §9.1`) y el código de error
  `WORKSPACE_RECOVERY_REQUIRED` bloquea escrituras hasta resolver.

La **orquestación** (qué pasos, en qué orden, ensamblar el `ChangeReceipt`, construir el envelope) la
conduce `lodestar-app` llamando a métodos de `Workspace`. El **cómo** (renames, fsync, journal) es de
`Workspace`.

---

## c) Tipos compartidos nuevos (invariante #4: UNA vez en `core::types`, sin DTO paralela)

Todos los tipos de dominio nuevos se congelan en `lodestar-core::types` (`crates/lodestar-core/src/types.rs`),
como el resto del contrato (§4.1/§4.4 de `ARCHITECTURE.md`). Esbozo y **reutilización de lo existente**:

| Tipo nuevo (REFACTOR §) | Forma propuesta | Reutiliza / se ancla en |
|---|---|---|
| `ConceptRevision` (§6.2) | `blake3:<hex>` del contenido en disco de un `.md` | **Ya existe**: `WriteOutcome.hash: [u8;32]` = `blake3::hash(raw)` (`bundle.rs:361`); el gate de la cache (`store/index.rs:89`). Se **eleva** de gate interno a identidad expuesta. |
| `WorkspaceRevision` (§6.3) | hash raíz determinista sobre `writableRoots`: ordenar paths normalizados → hash de cada contenido → combinar `path+hash` → hash raíz. **Independiente** de mtime, orden de fs, cachés, `.lodestar/runtime`, `referenceRoots`, ignorados | Función **pura nueva** en core sobre el `FileMap` filtrado; el orden lo da `RelPath: Ord` (ya es `Ord`, `types.rs:26`). Invariante #3: lo computa el core, no SQLite |
| `ConceptRef` (§6.1) | `{ path: RelPath, id: Option<ConceptId> }` con **path como identidad primaria**; `id` opcional/diferido | `RelPath` (`types.rs:27`). IDs obligatorios = no-goal (REFACTOR §16) |
| `ChangeSet` (§6.4) | `{ id, base_revision: WorkspaceRevision, operations: Vec<NormalizedOperation>, plan_hash, risk: RiskAssessment, semantic_diff: SemanticDiff, validation: ValidationReport, expires_at }` | `Mutation` (`types.rs:430`) es el antecedente de la lista de escrituras; `SemanticDiff` reutiliza `OkfDiff`/`diff_snap` (`diff.rs`); `ValidationReport` envuelve `Analysis`+`Vec<Check>` |
| `NormalizedOperation` (§6.4, §11.1) | enum con las 11 ops (`create`/`patch_frontmatter`/`replace_body`/`edit_section`/`replace_text`/`move`/`delete`/`add_relation`/`remove_relation`/`transition_status`/`apply_fix`) ya **resueltas** a escrituras concretas | `FrontmatterPatch` (`types.rs`, merge-patch RFC 7386, null-borra) para `patch_frontmatter`; `create_concept`/`merge_frontmatter` del core (`bundle.rs`) como base de la normalización |
| `RiskAssessment` (§6.4, §11.1) | `{ level: Low\|Medium\|High, reasons: Vec<String> }` derivado de backlinks/blast-radius del cambio | **Lógica pura nueva** en core, alimentada por `neighborhood`/blast-radius |
| `SemanticDiff` (§6.4, §11.1) | `{ created, modified, deleted, moved, frontmatterChanges, bodyChanges, relationChanges, diagnosticsIntroduced, diagnosticsResolved }` | **Reutiliza `OkfDiff`** (`diff.rs`, port de `diffSnap`); se amplía con `diagnosticsIntroduced/Resolved` (diff de `Vec<Check>` antes/después) |
| `ValidationReport` (§10, §11.1) | `{ conformant, summary{errors,warnings,info}, diagnostics: Vec<Check> }` | `Analysis.hard_fail`/`warn_count` (`types.rs`) + `Vec<Check>` |
| `ChangeReceipt` (§6.5) | `{ id, change_set_id, previous_revision, result_revision, changed_paths: Vec<RelPath>, semantic_diff }` | `Sha` como patrón de newtype para `ReceiptId`/`ChangeSetId` |
| **Envelope común** (§13) | `{ ok, workspaceRevision, summary, data, diagnostics, warnings, resourceLinks }` | **Ver decisión D3 abajo** — propongo que NO sea `core::types` |
| **Códigos de error** (§13) | enum estable de 15 códigos (`WORKSPACE_NOT_FOUND`…`INTERNAL_IO_ERROR`) | Patrón de `CheckCode` (`types.rs:102`, wire por `#[serde(rename)]`); mapea desde `CoreError`/`WorkspaceError` |

**Extensión del tipo `Check` (tensión de contrato — ver §Tensiones).** Los diagnósticos de REFACTOR
(§10) llevan `id`, `range{startLine,endLine}`, `related`, `fixes[{fixId,title,safe}]` que hoy `Check
{level, code, msg, targets}` (`types.rs:117`) no tiene. Propuesta: **extender la única definición** con
campos **opcionales/aditivos** (`id: Option<_>`, `range: Option<Range>`, `related: Vec<_>`,
`fixes: Vec<Fix>`), sin forkear (invariante #4). Los 15 checks OKF actuales los dejan vacíos.

### Decisión D3 — ¿dónde vive el envelope y el mapa de códigos de error?

- El **envelope** (`ok`/`summary` para el modelo/`resourceLinks`) tiene forma de **protocolo MCP**, no de
  dominio. Meterlo en `core::types` mete framing de transporte en el core puro.
- **Recomendación**: el **envelope vive en `lodestar-app`** (la capa de servicios que ambas fachadas
  comparten) — es "una vez" y compartido por mcp/cli, pero no es dominio. Los **códigos de error** SÍ van
  a `core::types` (son contrato estable como `CheckCode`, y la CLI los usa para su exit-code mapping).
  Esto respeta el invariante #4 (sin DTO **de dominio** paralela) sin ensuciar el core con framing.

> **Punto para el usuario (D3):** ¿envelope en `lodestar-app` (recomendado) o en `core::types` junto al
> resto del contrato? (Los códigos de error van a `core::types` en ambos casos.)

---

## d) Config nueva y separación canónico vs runtime

### d.1 Config: `.lodestar/config.yaml` (YAML) vs extender `lodestar.toml`

Hoy la config por-bundle es `lodestar.toml` (`config.rs:12`) con solo `[gate] block_warnings` (strictness)
e `[identity] name/email`. `REFACTOR.md §4.2` pide `.lodestar/config.yaml` con `workspace.writableRoots`
/ `referenceRoots` / `ignored`.

| Opción | Trade-offs |
|---|---|
| **A — Migrar a `.lodestar/config.yaml`** (YAML, todo unificado): `workspace.*` + `gate.*`; `identity` se conserva pero como sección **dormida** (git fuera de superficie) | Coherente con REFACTOR (YAML como el frontmatter del bundle); una sola config; pero rompe el `lodestar.toml` existente (migración) |
| **B — Extender `lodestar.toml`** con `[workspace]` writableRoots/referenceRoots | Cero migración; pero contradice la letra de REFACTOR (§4.2 dice YAML y `.lodestar/config.yaml`), y mezcla config en la raíz con `.lodestar/` |

**Recomendación: Opción A.** `.lodestar/config.yaml` unificado en YAML (idiomático con el bundle, que ya es
YAML frontmatter), con `workspace.{writableRoots,referenceRoots,ignored}` + `gate.{blockWarnings}` +
`transactions.{retainReceiptsFor,maximumReceipts}` (§11.3). `identity` se conserva bajo una sección
marcada como dormida (vcs desactivado). La lectura sigue el patrón de `Config::load` (workspace lee,
entrega datos; el core recibe la lista de roots ya resuelta).

> **Punto para el usuario (D4):** ¿migrar a `.lodestar/config.yaml` YAML unificado (recomendado, sigue
> REFACTOR §4.2) o extender el `lodestar.toml` existente para no migrar?

### d.2 Separación canónico vs runtime — **cambia el gitignore de `.lodestar/`**

Hoy **todo** `.lodestar/` está gitignored (es cache: `index.db`). El nuevo modelo parte `.lodestar/` en
**dos naturalezas** (`REFACTOR.md §4.1`):

- **Canónico / versionado** (entra a git, PERO **no** a `WorkspaceRevision` ni al índice de conceptos):
  `.lodestar/config.yaml`, `.lodestar/schema.yaml`, `.lodestar/templates/`. Son configuración del
  workspace, no conocimiento.
- **Runtime / desechable** (gitignored, como hoy `index.db`): `.lodestar/runtime/` (plans/, receipts/,
  journal, `audit.jsonl`) + `.lodestar/index.db`.

Consecuencia de diseño: el `.gitignore` que hoy ignora `.lodestar/` entero debe pasar a ignorar
**solo** `.lodestar/index.db` + `.lodestar/runtime/`. Esto toca la lógica de first-run/gitignore de E8 y
`vcs::ensure_cache_ignored` (`lib.rs:96`). **`WorkspaceRevision` excluye TODO `.lodestar/`** (canónico y
runtime): la revisión es solo de `writableRoots` de conocimiento (§6.3). Invariante #1 intacto: los `.md`
de conocimiento son la verdad; la config es config, no conocimiento.

> **Punto para el usuario (D5):** confirmar que `.lodestar/{config,schema}.yaml` y `.lodestar/templates/`
> pasan a estar **versionados** (dejan de estar gitignored), mientras `.lodestar/runtime/` + `index.db`
> siguen gitignored. (Recomendado; es lo que pide REFACTOR §4.1.)

---

## e) Superficie MCP 13 → 10

### e.1 Tabla de migración (desde las 13 tools reales de `crates/lodestar-mcp/src/tools.rs:25`)

| Tool actual (13) | Destino (10) | Nota |
|---|---|---|
| `find_backlinks` | `graph_query(operation="backlinks")` | Consolida en graph_query |
| `find_orphans` | `graph_query(operation="orphans")` + filtro `is:orphan` de `knowledge_search` | |
| `find_dangling` | `graph_query(operation="dangling")` + `knowledge_check` | |
| `neighborhood` | `graph_query(operation="neighborhood")` | Reutiliza `Bundle::neighborhood` (`bundle.rs:254`) |
| `conformance_check` | `knowledge_check` | Scopes workspace/concept/paths/affected (§10) |
| `query` | `knowledge_search` | + filtros, snippets, paginación por cursor |
| `create_concept` | `change_plan` + `change_apply` (op `create`) | Ya no escritura directa |
| `update_frontmatter` | `change_plan` + `change_apply` (op `patch_frontmatter`) | `FrontmatterPatch` se reutiliza |
| `generate_index` | **CLI** `lodestar index` (existe) + auto-regen dentro de `change_apply` | Ver D6 |
| `generate_tag_indexes` | **CLI** `lodestar tags` (existe) + auto-regen en `change_apply` | Ver D6 |
| `history` | **ELIMINADA** | Git fuera de superficie (decisión cerrada #1) |
| `last_conforming_commit` | **ELIMINADA** | idem |
| `commit` | **ELIMINADA** | idem |
| — | **NUEVAS**: `workspace_status`, `knowledge_get`, `schema_inspect`, `impact_analyze`, `change_plan`, `change_apply`, `change_revert` | |

Las 10 finales (`REFACTOR.md §8`): READ = `workspace_status`, `knowledge_search`, `knowledge_get`,
`schema_inspect`, `graph_query`, `impact_analyze`; VERIFY = `knowledge_check`; CHANGE = `change_plan`,
`change_apply`, `change_revert`. `impact_analyze` reutiliza el **blast-radius** del store
(`synth.rs:110`, `Store::blast_radius` `lib.rs:296`) y `neighborhood` (`graph.rs`/`bundle.rs:254`).

### e.2 Perfiles (§12) y política

- `readonly` = las 7 tools de lectura/verificación. `standard` = añade `change_plan`/`change_apply`/
  `change_revert`. Se elige **al arrancar el servidor** (`--profile readonly|standard`), no por llamada.
- Política de conformidad **al arrancar** (`lodestar-mcp --policy strict`, `strict` por defecto,
  `REFACTOR.md §11.2`): no se expone `allow_nonconformant` por llamada (invariante de seguridad §14).
  Hoy el MCP toma solo `<bundle>`; se añaden los flags `--profile`/`--policy`.

### e.3 Transporte y `outputSchema`

`DECISIONES.md §3` recomienda **mantener el stdio propio** hasta tener un cliente que exija `rmcp`. El
contrato §13 de REFACTOR pide `inputSchema` **y `outputSchema`** + fixtures + pruebas de compatibilidad.
`outputSchema` **no** exige `rmcp`: la feature `schemars` ya está preparada en el core
(`ARCHITECTURE.md §10` fila 14; `types.rs:12` macro `schema_derive!`).

**Recomendación**: mantener el **transporte stdio** (DECISIONES §3) **y** activar `outputSchema` derivado
con `schemars` en las 10 tools (el contrato lo pide; la feature ya existe). `rmcp` sigue **diferido**.
`contracts/mcp.yml` se **reescribe** de 13→10 y el guardián de contrato (`/contrato --check`) lo vigila.

> **Punto para el usuario (D6):** (a) ¿`generate_index`/`generate_tag_indexes` quedan **solo en CLI** +
> auto-regeneración dentro de `change_apply` cuando el cambio afecta a index/tags (recomendado), o se
> conserva alguna tool MCP de generación? (b) ¿confirmas **stdio + outputSchema vía schemars**, con `rmcp`
> diferido?

---

## f) Seguridad (§14)

Confirmaciones (sin cambio de invariante; **#6 sigue vigente**):

- **`RelPath` sigue siendo el chokepoint sintáctico** (`types.rs:33`): rechaza absolutas (POSIX y unidad
  Windows `C:`), `..`, backslashes y vacío. **Se refuerza** con una comprobación **semántica de nivel
  workspace** (nueva, aditiva): (1) que el path resuelto cae bajo un `writableRoot` para escribir, y bajo
  un root visible para leer; (2) **guarda de symlinks** — canonicalizar y verificar contención dentro del
  root, rechazando si el enlace escapa (`REFACTOR.md §14`). Nota: la canonicalización en Windows usa la
  ruta verbatim, quirk ya conocido y resuelto en el arnés diferencial (commit `770cff7`).
- **Solo se escribe dentro de `writableRoots`**; `referenceRoots` son visibles pero **inmutables** por el
  MCP (§4.2). El core valida que los paths externos (`implemented_by`/`verified_by`) existen, sin editarlos.
- **Sin ejecución de comandos, sin red, sin git en la superficie**: el crate `vcs` (que era el único que
  hacía shell-out/red) queda **sin consumidores** (decisión #1) → la superficie no ejecuta procesos ni
  toca la red. Esto de hecho **simplifica** el threat model de `ARCHITECTURE.md §12`.
- **Auditoría** local en `.lodestar/runtime/audit.jsonl` (§14): no es conocimiento canónico, es runtime.

Ninguna de estas es una decisión abierta; se declaran en la adenda.

---

## g) Mapa de las 6 fases → épicas (esbozo para la Fase B)

Épicas existentes: `00`–`08` (todas hechas). Las nuevas arrancan en **`09`**. Esbozo (la descomposición
en historias es la Fase B, tras ratificar):

| Épica | Fase REFACTOR | Objetivo | Frontera |
|---|---|---|---|
| **09 — Reducción de alcance** | Fase 0 (§16) | Retirar git de la superficie (borrar `cli/git.rs`, tools MCP `history`/`last_conforming_commit`/`commit`); **congelar la UI** en `.claude/`+`CLAUDE.md`+`docs/WORKFLOWS.md`; introducir `.lodestar/config.yaml` (writableRoots/referenceRoots/ignored) + separación canónico/runtime; escribir la **adenda §19** y reposicionar README/CLAUDE | `mcp.yml` recorta 3 tools; `ipc.yml` no cambia (UI congelada) |
| **10 — Esquemas + lectura headless** | Fase 1 (§16) | `core::schema` (DocType/requiredFields/allowedStatuses/lifecycle/templates) puro; tipos `ConceptRevision`/`WorkspaceRevision`/`ConceptRef`; extensión de `Check` (id/range/fixes); envelope + códigos de error; crate `lodestar-app`; tools `workspace_status`/`knowledge_search`/`knowledge_get`/`schema_inspect`/`knowledge_check` | `mcp.yml` reescrito (parte lectura) |
| **11 — Grafo e impacto** | Fase 2 (§16) | `graph_query` (consolida backlinks/orphans/dangling/neighborhood + path_between/cycles/components); `impact_analyze` (reusa `blast_radius` `synth.rs:110` + `neighborhood`); typed relations + validación de paths de `referenceRoots` | `mcp.yml` (2 tools) |
| **12 — Planificación** | Fase 3 (§16) | `ChangeSet`/`NormalizedOperation`/`RiskAssessment`/`SemanticDiff`/`ValidationReport`; `change_plan` (normaliza+simula+valida sin escribir; persiste plan en `.lodestar/runtime/plans/`); las 11 operaciones; optimistic concurrency (`expectedRevision`/`expectedWorkspaceRevision`) | `mcp.yml` (change_plan) |
| **13 — Publicación recuperable** | Fase 4 (§16) | Modelo transaccional en workspace (staging, write-ahead journal, locks, copias de recuperación, crash recovery); `change_apply` (proceso 15 pasos §11.2); `change_revert`; `ChangeReceipt`; `audit.jsonl` | `mcp.yml` (change_apply/revert) |
| **14 — Integración software + evaluación** | Fases 5+6 (§16) | Validación de paths de código en CI; knowledge checks en CI; config por proyecto; instrucciones para agentes; benchmarks (Claude Code/Codex), tokens, concurrencia, recuperación (§17 benchmark funcional) | — |

Notas de orden: 09 es prerrequisito de todo (retira git, define config/runtime, escribe la adenda). 10
habilita 11–13 (schemas y revisiones son la base de impacto y planificación). 12 depende de 11
(impact_analyze alimenta el `RiskAssessment` del plan). 13 depende de 12 (aplica planes). 14 cierra.
El detalle de historias, dependencias sin ciclos y orden de construcción es la Fase B.

---

## Tensiones con invariantes detectadas (con resolución propuesta)

Ninguna es un bloqueo; todas tienen resolución que **preserva** el invariante:

1. **Invariante #1 (los `.md` son la única verdad) vs `.lodestar/` versionado.** REFACTOR versiona
   `config.yaml`/`schema.yaml`/`templates/` dentro de `.lodestar/`. **Resolución**: es *configuración del
   workspace*, no *conocimiento*; queda fuera de `WorkspaceRevision` y del índice de conceptos. Los `.md`
   de `writableRoots` siguen siendo la única fuente de verdad del conocimiento. Staging/journal/receipts/
   audit van a `.lodestar/runtime/` (desechable). Invariante intacto.

2. **Invariante #4 (un contrato de tipos, sin DTO paralela) vs el envelope + `Check` enriquecido.**
   **Resolución**: (a) el envelope es *framing de protocolo*, no dominio → vive en `lodestar-app`, una
   vez, compartido (decisión D3). (b) `Check` se **extiende** con campos opcionales (`id`/`range`/
   `related`/`fixes`) en su **única** definición de `core::types`, no se forka. (c) los códigos de error
   van a `core::types` como `CheckCode`.

3. **`CheckCode` congelado (15 + OKF-CONFLICT) vs los diagnósticos schema-driven de REFACTOR
   (`REL-TARGET-MISSING`, required-field-missing, invalid-status…).** `ARCHITECTURE.md §12` manda
   "`CheckCode` **aditivo-solo** con deprecación explícita" y la i18n está **keyed por código**.
   **Resolución propuesta**: **añadir familias acotadas de variantes estáticas** (p. ej. `SCHEMA-REQFIELD`,
   `SCHEMA-STATUS`, `REL-TARGET`, `REL-CARD`, `REL-TYPE`) — aditivo, con clave i18n por código — **en vez
   de** un espacio de códigos dinámico abierto (que rompería la i18n keyed y la disciplina de contrato
   congelado). El *qué* concreto (qué campo, qué relación) va en `targets`/`msg`/`related`, no en una
   explosión de códigos. **Esto necesita tu criterio** (afecta al contrato congelado).

4. **Invariante #5 (un único escritor) vs escritura transaccional multi-fichero.** **Resolución**: el lote
   transaccional se publica por el **mismo** único escritor (`write_atomic` en bucle + journal), no por un
   segundo escritor. Staging vive en `.lodestar/runtime/staging/` (no es el árbol canónico, no es "el
   escritor"). El crash-recovery también pasa por el único escritor. Invariante intacto.

5. **Invariante #6 (`RelPath` chokepoint) vs writableRoots + symlinks.** **Resolución**: `RelPath` sigue
   siendo el chokepoint **sintáctico**; se **añade** (aditivo) una comprobación **semántica** de
   contención en workspace (bajo un root escribible/visible + guarda de symlink por canonicalización). No
   sustituye a `RelPath`; lo complementa.

6. **`lodestar check --staged/--rev/--range` (git-tree-scoped, `ARCHITECTURE.md §13.5`) sin consumidor de
   superficie.** El usuario retira los subcomandos git de `cli/git.rs`, pero `check` vive en
   `commands.rs`. **Resolución propuesta**: `check` **permanece** como la puerta de CI, pero **operando
   sobre el working tree / scope de workspace** (equivale al futuro `knowledge_check` en CLI, REFACTOR
   Fase 5). Las variantes `--staged/--rev/--range` (que llaman a `vcs`) quedan **diferidas junto al crate
   vcs dormido** (el crate se conserva, el flag no se expone). **Necesita tu confirmación** (¿se retiran
   también esos flags de `check` o se conservan aunque el resto de git salga?).

---

## Índice de decisiones a ratificar (para AskUserQuestion)

- **D0** — Forma de la adenda: **§19 nueva + nota en §13** (recomendado) vs reescribir §13 in situ.
- **D1** — Capas nuevas: **Opción C** (mecánica transaccional en `workspace` + `lodestar-app` fino,
  recomendado) vs A (todo en `lodestar-app`) vs B (ampliar `workspace`, cero crates).
- **D3** — Envelope: **en `lodestar-app`** (recomendado) vs en `core::types`. (Códigos de error → siempre
  `core::types`.)
- **D4** — Config: **migrar a `.lodestar/config.yaml` YAML unificado** (recomendado) vs extender
  `lodestar.toml`.
- **D5** — Confirmar que `.lodestar/{config,schema}.yaml` + `templates/` pasan a **versionados** y
  `.lodestar/runtime/` + `index.db` siguen **gitignored** (recomendado).
- **D6** — MCP: (a) generadores **solo CLI + auto-regen en `change_apply`** (recomendado) vs conservar
  tool; (b) confirmar **stdio + outputSchema vía schemars**, `rmcp` diferido.
- **D-CheckCode** (tensión #3) — Diagnósticos de schema/relaciones como **familias acotadas de variantes
  estáticas de `CheckCode`** (recomendado) vs espacio de códigos dinámico.
- **D-check** (tensión #6) — ¿`lodestar check` conserva `--staged/--rev/--range` (aunque git salga de la
  superficie) o esos flags se **difieren con el crate vcs dormido** (recomendado)?

Confirmadas (no requieren criterio, se declaran en la adenda): schema en `core` puro (§b.2);
modelo transaccional en `workspace` (§b.3); reutilización de `OkfDiff`/`blast_radius`/`neighborhood`/
`Mutation`/`RelPath`/blake3 (§c); seguridad §14 (§f).

---

*Fin de la propuesta. Esperando ratificación del usuario antes de escribir la adenda en
`ARCHITECTURE.md`, anotar el cierre en `DECISIONES.md` (§ del giro) y pasar a la Fase B.*
