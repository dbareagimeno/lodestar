# Changelog

Todos los cambios notables de este proyecto se documentan en este archivo.

El formato se basa en [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/)
y el proyecto sigue [Versionado Semántico](https://semver.org/lang/es/).

## [No publicado]

## [0.3.0] - 2026-07-24

**Migración de OKF a workspaces Markdown universales** (`ARCHITECTURE.md §20`, ratificado el
2026-07-23; épicas E15–E22; fuente `docs/REFACTOR_PHASE_2.md`). lodestar deja de exigir el formato
documental propio **OKF** y pasa a operar sobre **cualquier red de ficheros Markdown contenida en un
proyecto**: `cd my-project && lodestar-mcp` funciona sin `init`, sin `.lodestar/`, sin `index.md`,
sin frontmatter obligatorio.

> **⚠️ Versión INCOMPATIBLE con v0.2.x.** El modelo documental, la superficie MCP y el DDL del store
> cambian. La cache `.lodestar/index.db` se reconstruye automáticamente; los `.md` OKF existentes
> siguen siendo Markdown válido (ver `migrate-from-okf --dry-run`), pero pierden la semántica especial
> de OKF.

### Cambiado

- **El `cwd` es el workspace** (E15): `lodestar-mcp` arranca desde cualquier directorio (`--root` para
  fijarlo); descubrimiento recursivo de todos los `**/*.md` respetando `.gitignore`/`.lodestarignore`.
- **Modelo documental genérico** (E16): el frontmatter es **YAML arbitrario** con sus tipos reales
  (sin campos conocidos, sin `type` obligatorio); ningún nombre de fichero (`index.md`, `README.md`,
  `log.md`) activa reglas especiales; título derivado (`frontmatter.title` → primer H1 → nombre del
  fichero); `patch_frontmatter` **quirúrgico** que no reescribe el bloque salvo que sea necesario y
  no puede destruir un frontmatter ilegible.
- **Enlaces Markdown estándar** (E17): resueltos **solo por path** (inline, de referencia, con
  fragmento, anchors, externos), clasificados en `LinkTarget` (documento / fichero del proyecto /
  externo / self-anchor / roto / escapa). Grafo universal: todos los `.md` son nodos.
- **Store v2** (E18): DDL `documents`/`metadata`/`links`/`diagnostics` sin columnas OKF; metadata
  indexada por field path recursivo con su tipo; FTS sin campos privilegiados.
- **Lenguaje de consulta tipado** (E19): `where` textual y `filter` JSON producen el mismo AST y el
  mismo resultado; dot-notation, listas, existencia, namespaces `document.*`/`graph.*`; **sin coerción
  implícita** (`priority >= "high"` es un error de tipo). Sustituye la DSL de subcadena.
- **Validación genérica** (E20): diagnósticos mínimos de `§20.9` (nada de «falta `type`»);
  `metadata_inspect` (catálogo de propiedades e inspección de campo) sustituye a `schema_inspect`;
  política `rejectNewErrors`/`allowExistingErrors` (se puede reparar un repo que ya tiene problemas).
- **Operaciones transaccionales universales** (E21): las **7** de `§20.11` (`create`,
  `patch_frontmatter`, `replace_body`, `replace_text`, `edit_section`, `move`,
  `delete`), selecciones masivas por consulta, y `move` que reescribe
  los backlinks relativos (incluidas las definiciones de referencia). El motor transaccional
  (staging/journal/locks/recovery/receipt/revert) **no cambia**.

### Retirado

- **OKF como formato obligatorio**: fuera `core::schema` (`DocType`, `requiredFields`,
  `allowedStatuses`, relaciones tipadas, `.lodestar/schema.yaml`), los códigos `OKF-*`/`SCHEMA-*`/
  `REL-*`, `in_index`/`okf_version` como semántica, y las 5 operaciones semánticas (`add_relation`,
  `remove_relation`, `transition_status`, `deprecate`, `replace_concept`).
- **git**: el crate `lodestar-vcs` se **borra** del repo (era una capacidad dormida).
- **Generadores e intercambio**: `lodestar init`/`index`/`tags`/`export`/`import`.
- **El prototipo JS** como spec de comportamiento (la spec pasa a ser `docs/REFACTOR_PHASE_2.md`).
- Terminología OKF de la API pública: `Concept`→`Document`, `Bundle`→`Workspace` como **concepto**
  (`ARCHITECTURE §20.3`) y `DocumentSet` como **tipo** del core que lo sustituye (`§20.4`),
  `Conformance`→`Validation`, `CONCEPT_NOT_FOUND`→`DOCUMENT_NOT_FOUND`.

### Añadido

- **`lodestar migrate-from-okf --dry-run`**: diagnóstico de cortesía que detecta convenciones OKF
  legadas (`index.md` raíz, índices anidados, `okf_version`, índices de tags) **sin modificar ningún
  fichero**.

### Cierre de la migración (E23)

Épica de cierre, abierta por la revisión de la PR #17 (2026-07-25), que salda los defectos que la
migración dejó vivos **antes** de publicar. Se recogen aquí porque v0.3.0 no llegó a publicarse sin
ellos.

**Corregido**

- **Abrir un workspace ya no modifica el proyecto.** `lodestar check` y arrancar `lodestar-mcp`
  —incluso en perfil `readonly`— reescribían el `.gitignore` y creaban `.lodestar/runtime/` antes de
  leer nada. Ahora abrir es **hermético**: los dos efectos ocurren en los cuatro puntos que van a
  escribir de verdad.
- **`lodestar check` y `knowledge_check` daban veredictos contradictorios** sobre el mismo
  workspace: la validación ignoraba la sección `validation` de la config y los diagnósticos de
  descubrimiento.
- **`recovery.pendingTransaction` era un `false` literal**: tras un crash, la primera tool que
  llamaba un agente le mentía.
- **No se podía mover una nota que enlazara a sus vecinas**: los salientes del documento movido no
  se recalculaban y el gate lo veía como errores nuevos.
- **`create` escribía `type: ''`** (residuo OKF) y un `title` que nadie pidió.
- **Un lock huérfano era irrecuperable**: un proceso muerto por SIGKILL cerraba la base a la
  escritura para siempre. Ahora se reclama por TTL + PID.
- **NFC/NFD**: un enlace correcto tumbaba el CI en macOS. Resolución tolerante con aviso, sin
  normalizar la ruta canónica.
- **Corrupción real**: reescribir el cuerpo de un documento **sin frontmatter** le inyectaba
  `---\n{}\n---`, así que mover un documento corrompía de una tacada todos sus enlazantes sin
  frontmatter.

**Añadido**

- **Proyección de frontmatter en `knowledge_search`**: `include: ["frontmatter.status"]` (y
  anidados, `frontmatter.owner.name`) devuelve esos campos en cada resultado, con sus tipos YAML
  reales. Antes, ver el `status` de 30 resultados costaba 30 `knowledge_get`.
- **`workspace_status` lista los recibos** (`receiptId`, `changeSetId`, `resultRevision`,
  `changedPathCount`): perder el `receiptId` dejaba el undo inalcanzable pese a estar persistido.
- **`metadata_inspect` explota las listas** al contar valores, así que ya se puede obtener el
  vocabulario de tags de una base.
- El `inputSchema` de `change_plan` declara **los 18 parámetros** que el código lee, no 4.

**Retirado**

- **`apply_fix`** (las ops universales quedan en **7**): sin productor de `Fix` desde E20-H03 fallaba
  siempre, y encima devolvía `DOCUMENT_NOT_FOUND`. El lado de lectura (`fixes`,
  `includeSuggestedFixes`) se conserva. Ver `docs/PROPUESTA_FIXES.md`.
- **`sort` en `knowledge_search`**: se aceptaba y se ignoraba en silencio. El orden es siempre
  determinista (score desc, path asc).
- **`retarget` y `create_stub`** como políticas de `delete`: se aceptaban **sin ejecutarse**,
  dejando los enlaces entrantes rotos.
- **`implemented_by`/`verified_by`** como claves de frontmatter privilegiadas, y con ellas
  `include:["externalReferences"]` en `knowledge_get`. Ningún nombre de campo tiene ya semántica
  impuesta. Apuntar a código sigue siendo posible con un enlace Markdown normal.
- **`Workspace::open_ephemeral`**: quedó idéntico a `open` cuando abrir pasó a ser hermético.

**Cambiado (wire)**

- `conformant` → `valid` · `requireConformantResult` → `requireValidResult` · `allowNonconformant` →
  `allowInvalid` · `NONCONFORMANT_RESULT` → `INVALID_RESULT`; y la salida humana de `check`,
  `CONFORME` → `VÁLIDO`. El catálogo de errores sigue teniendo 16 filas: se sustituyó una, no se
  añadió ninguna.
- **Las comparaciones de fecha son lexicográficas** y ahora está declarado: `serde_yaml` 0.9 no tipa
  timestamps, así que un `2026-07-23` sin comillas es un string. Con ISO-8601 bien formado coincide
  con el orden cronológico; con formatos mixtos, no.

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
