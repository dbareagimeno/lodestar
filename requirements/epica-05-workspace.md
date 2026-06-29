# E5 — `lodestar-workspace` (el handle unificado)

> **Fase**: `§14.5`. **Objetivo de la épica**: la crate **glue** que compone `core` (puro) + `store` +
> `vcs` y expone el handle `Workspace` que ven las 3 fachadas. Es el **ÚNICO escritor** (commit/restore
> pasan por aquí), dueño del **único watcher por proceso** y del **bus de eventos**. Sin tokio.
> Referencias: `ARCHITECTURE.md §6`, `§9`, `§13.6.1` (checkpoint), `§10` filas 2/8/12/16.

**Invariante de la épica**: los comandos **nunca** escriben la cache; escriben el `.md` (atómico
temp+rename) y el watcher reconcilia. Las operaciones git que reescriben el working tree
(restore/switch/merge) lo hacen por el **único escritor** + **checkpoint** automático.

---

### E5-H01 — `Workspace::open` / `open_ephemeral` (compone core+store+vcs)
- **Objetivo**: abrir el handle unificado: cache + watcher + `Vcs::discover`, o efímero sin cache.
- **Referencias**: `ARCHITECTURE.md §6`, `§3`.
- **Alcance**:
  - `open(root) -> Result<Self, WorkspaceError>`: abre/crea la cache (`store`), arranca el **único watcher**,
    `Vcs::discover` (puede ser `None` = modo "activar git").
  - `open_ephemeral(root) -> Result<Self,_>`: sin cache (CLI hermético) — construye `Bundle` desde disco directo.
  - **Un solo watcher por proceso** que posee el **único escritor** de SQLite.
  - **Un bundle por proceso** (`§12`): lockfile que elige un único indexador cuando GUI y MCP abren el mismo bundle.
- **Criterios de aceptación**:
  - `open` deja un watcher corriendo y la cache lista; `open_ephemeral` no crea `.lodestar/index.db`.
  - Abrir el mismo bundle desde dos procesos respeta el lockfile (un solo indexador).
- **Dependencias**: E3-H01, E3-H04, E4-H01.
- **Pruebas**: open/ephemeral; lockfile con dos handles.

### E5-H02 — Único escritor: escritura atómica `temp+rename` + gate de echo por hash
- **Objetivo**: el camino de escritura único que todos los comandos usan; el watcher reconcilia.
- **Referencias**: `ARCHITECTURE.md §6`, `§9`, `§10` fila 8.
- **Alcance**:
  - Escritura atómica (temp file + rename) del `.md`; **los comandos nunca escriben la cache directamente**.
  - **Echo-suppression**: el `hash` blake3 de la cache es la única autoridad; cada DTO de lectura/escritura
    expone el `hash` para que el editor distinga su propio echo de una edición externa.
  - Aplicar una `core::Mutation`/`WriteOutcome` = escribir cada `.md` por este camino.
- **Criterios de aceptación**:
  - Una escritura nunca toca `.lodestar/index.db` directamente; el watcher hace el upsert.
  - El `hash` devuelto por la escritura coincide con el que el watcher computa (echo suprimido).
  - Una escritura interrumpida no deja un `.md` a medias (atomicidad temp+rename).
- **Dependencias**: E5-H01, E3-H04, E1-H13.
- **Pruebas**: escritura atómica; echo suprimido; crash-injection deja el fichero íntegro o intacto.

### E5-H03 — `subscribe` + `snapshot` (BundleSnapshot: files + analysis + graph juntos)
- **Objetivo**: la suscripción al bus y el snapshot unificado que empuja la fachada.
- **Referencias**: `ARCHITECTURE.md §6`, `§9`, `§8` (snapshot empujado), `§11` (eventos delta).
- **Alcance**:
  - `subscribe() -> crossbeam::Receiver<IndexEvent>`.
  - `snapshot() -> BundleSnapshot` con files + `Analysis` + `GraphModel`, todo junto.
  - `workspace` recomputa `Analysis` (core) + snapshot tras cada `IndexEvent`.
  - A escala: preparar el camino para **eventos delta** (no full-snapshot) y proyecciones SQL (`§11`); v1 puede full-snapshot.
- **Criterios de aceptación**:
  - Tras una edición, un suscriptor recibe `IndexEvent` y `snapshot()` refleja el cambio.
  - El `BundleSnapshot` es serializable camelCase (consumible por Tauri y por el golden cross-fachada).
- **Dependencias**: E5-H01, E3-H06, E1-H07, E1-H10.
- **Pruebas**: editar → evento → snapshot poblado (smoke del `§10` fila 7).

### E5-H04 — Delegaciones de lectura/escritura semántica al core
- **Objetivo**: exponer en `Workspace` los métodos que delegan en core y aplican por el único escritor.
- **Referencias**: `ARCHITECTURE.md §6`, `§4.2`, `§10` fila 12.
- **Alcance**:
  - Lecturas: `backlinks`/`neighborhood`/`query`/`conformance`/`list_concepts`/`graph_model` (delegan en core/store).
  - Escrituras: `create_concept`/`merge_frontmatter`/`add_log_entry` (core computa `WriteOutcome` → único escritor).
  - **Generadores**: `generate_index`/`generate_tag_indexes` aplican la `Mutation` del core por el único camino
    y calculan **`{written, removed, unchanged}`** diffeando contra disco (de ahí el `--check` de CI, `§10` fila 12).
  - `export` delega en `core::export_zip`.
- **Criterios de aceptación**:
  - Un `create_concept` no-conforme devuelve `rejected` sin escribir; uno conforme escribe el `.md` por el único escritor.
  - `generate_index` devuelve `{written,removed,unchanged}` correctos contra disco.
- **Dependencias**: E5-H02, E1-H13, E1-H14, E1-H16.
- **Pruebas**: rechazo no-conforme; generadores con conteo; export.

### E5-H05 — Operaciones git de la workspace con **checkpoint** (ship-blocker `§13.6.1`)
- **Objetivo**: `commit`/`restore`/`switch_branch`/`merge`/`create_branch`/`init` por el único escritor, sin perder trabajo.
- **Referencias**: `ARCHITECTURE.md §6`, `§13.6.1`, `§9` (Git), `§10` fila 16.
- **Alcance**:
  - `restore`/`switch_branch`/`merge` toman el file-map destino de `vcs` (E4-H06), computan un `core::Mutation`
    (diff vs working tree) y lo aplican por el **único escritor** (lote auto-originado que el reconcile absorbe).
  - **Checkpoint automático**: si hay cambios sin commitear, primero un **commit de checkpoint** (trabajo no
    perdido → "un commit más al que volver"); **excluye el `log.md` curado**; **regenera** `index`/`tags` tras aplicar.
  - `commit`: la workspace **corre `check` ella misma antes** (los commits libgit2 no disparan hooks, `§13.5`);
    si `RepoState != Clean` (merge en curso), **niega** el commit y avisa "resolviendo conflicto" (`§13.6.3`).
  - `commit` opcionalmente anexa al `log.md` (opt-in del diálogo) en el **mismo** commit (`§13.7`).
  - Update **optimista** del pill con el `Sha` devuelto (nunca espera el echo, `§13.6.4`).
  - `create_branch`/`branches`/`vcs_log`/`vcs_diff`/`last_conforming` son lecturas; `pull`/`push` delegan en el
    subproceso `git` (escritor externo que el watcher/ref-watch absorben).
- **Criterios de aceptación**:
  - Un `restore`/`switch`/`merge` con cambios sin commitear crea un checkpoint **antes** de reescribir el working tree.
  - El checkpoint NO incluye `log.md`; tras aplicar, `index`/`tags` quedan regenerados.
  - `commit` sobre `RepoState::Merging` se **niega** con mensaje claro.
  - `commit` corre `check` y rechaza un árbol con hard-fail (salvo override explícito).
- **Dependencias**: E5-H02, E5-H04, E4-H05, E4-H06, E4-H09.
- **Pruebas**: checkpoint preserva trabajo; commit negado en merge; regeneración post-restore; commit corre check.

### E5-H06 — `WorkspaceError` unificado + supervisión del watcher
- **Objetivo**: el error que envuelve `CoreError`+`CacheError`(+vcs) y la supervisión del watcher.
- **Referencias**: `ARCHITECTURE.md §6`, `§12` (errores: supervisar el watcher).
- **Alcance**:
  - `WorkspaceError` que envuelve `CoreError` + `CacheError` (+ errores de vcs); las fachadas lo mapean a exit
    code / toast con **código estable** (`§12`).
  - **Supervisar el watcher**: panic → restart + banner; **nunca** UI obsoleta en silencio.
- **Criterios de aceptación**:
  - Cada `WorkspaceError` lleva un código estable mapeable a exit code y a `{code,message}` de Tauri.
  - Matar el thread del watcher provoca restart + señal de banner (no silencio).
- **Dependencias**: E5-H01, E1-H02.
- **Pruebas**: mapeo de errores; inyección de panic en el watcher → restart observable.

### E5-H07 — `reindex` real + `reconcile` al enfocar (cerrando E2-H06)
- **Objetivo**: reconstruir la cache bajo demanda y reconciliar refs/bytes al recuperar foco.
- **Referencias**: `ARCHITECTURE.md §5` (`reconcile_all`), `§9`, `§13.6.4` (reconcile al enfocar).
- **Alcance**:
  - `reindex`: fuerza un cold rebuild (`store`) por el único escritor; completa el stub de E2-H06.
  - Hook `reconcile()` que la fachada llama al enfocar la ventana (absorbe cambios de `pull`/refs externas).
- **Criterios de aceptación**:
  - `reindex` deja la cache idéntica a un rebuild desde cero.
  - `reconcile()` tras un `git pull` externo deja cache y pill consistentes.
- **Dependencias**: E5-H01, E3-H03, E4-H08.
- **Pruebas**: reindex == rebuild; reconcile tras pull externo.
