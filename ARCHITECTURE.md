# lodestar — Arquitectura

> Editor local-first de bases de conocimiento en formato **OKF** (Open Knowledge Format):
> un directorio de `.md` con frontmatter YAML. "Solo ficheros": legible por humanos y
> agentes, versionable en git, sin SDK. Este documento define la implementación real a
> partir del prototipo de `prototype/index.html`.

## 1. Stack

| Capa | Elección | Por qué |
|---|---|---|
| Shell de escritorio | **Tauri v2** | Rust es el backend de primera clase; binario ~5–10 MB; webview del SO |
| Backend / lógica | **Rust** (`lodestar-core` puro) | Toda la semántica OKF en un sitio, testeable sin GUI/DB/runtime |
| Frontend | **Svelte 5 + Vite** | Runtime mínimo; porta el prototipo verbatim; mejor que Astro (estático) o React (más pesado) |
| Cache / índice | **SQLite + FTS5** (`rusqlite`) | Cold-start y full-text a escala; **derivado y desechable**, nunca la verdad |
| Watcher | **`notify`** | Convergencia multi-escritor (app, MCP, edición a pelo, `git pull`) |
| Versionado | **git** (`git2`/libgit2) | "Versiones" = commits; historial, diff OKF y conformidad-por-commit; local-first |
| Fachada agentes | **MCP** (`rmcp`, stdio) | Expone *semántica* a Claude Code, no CRUD de ficheros |
| Fachada CI | **CLI** (`clap`) | `lodestar check` como puerta de CI con exit codes |

## 2. Principios (no negociables)

1. **Los `.md` en disco son la única fuente de verdad.** Todo lo demás se deriva y se
   puede reconstruir. Git, edición externa y agentes convergen porque todos escriben ficheros.
2. **`lodestar-core` es puro.** Sin `tauri`, sin `rusqlite`, sin `notify`, sin tokio.
   Solo modelo + lógica OKF. Unit-testeable y (potencialmente) wasm-able.
3. **Una sola verdad computada.** Backlinks, huérfanos, conformidad, query y grafo se
   computan con la **misma lógica de `lodestar-core`** en las tres fachadas. La cache SQLite
   refleja esas computaciones por velocidad/FTS y se **verifica idéntica con un test de paridad**.
   Cuando podrían discrepar, gana el core; `lodestar check` reconcilia antes de leer.
4. **Un solo contrato de tipos.** `Check`, `Severity`, `Analysis`, `GraphModel`, etc. se
   definen **una vez** en `lodestar-core` y cruzan a Tauri/MCP/CLI **sin capa DTO paralela**.
   El `.d.ts` de TypeScript se **genera** desde los tipos Rust (ts-rs/specta).

---

## 3. Mapa de crates (Cargo workspace)

La decisión clave que resuelve el conflicto "¿dónde vive el `Workspace`?": **el core
permanece puro** y se introduce una **crate de orquestación** que compone core + store + watcher.

```
crates/
  lodestar-core/        # PURO. modelo, conformidad, links, query, grafo, generación, export,
                        #       diff semántico OKF. Sin I/O, sin DB, sin git, sin runtime.
        ▲          ▲
  lodestar-store/    lodestar-vcs/   # store: rusqlite+FTS5+watcher notify, dueño del DDL .lodestar/index.db.
        ▲          ▲                 # vcs:   git2/libgit2, dueño de git (status/log/diff/commit/restore/init,
        │          │                 #        lee árboles de commit a file-maps, ref-watch). NO toca el working tree.
  lodestar-workspace/   # GLUE. Compone core + store + vcs. Handle `Workspace` unificado + bus de eventos.
        ▲               #       Único escritor (commit/restore pasan por aquí). Sin tokio.
        │  ▲  ▲
  src-tauri/  lodestar-cli/  lodestar-mcp/   # 3 fachadas finas sobre `Workspace`.
```

- **No existe `lodestar_core::Workspace`** (arrastraría rusqlite/notify al core). El handle
  unificado vive en `lodestar-workspace`. Las tres fachadas dependen de esa crate, no de `store`.
- **`rusqlite` vive SOLO en `lodestar-store`.** El motor de grafo/conformidad del core opera
  sobre el mapa de ficheros en memoria (o un trait `ConceptStore`), **nunca declara DDL**.
- **`git2`/libgit2 vive SOLO en `lodestar-vcs`** (igual que rusqlite en store). El core no sabe de git;
  el diff *semántico* OKF sí es lógica pura del core. libgit2 (no shell a `git`) por seguridad: abrir un
  bundle ajeno no debe ejecutar sus hooks/aliases/config. Git history y la cache SQLite tienen ciclos de
  vida opuestos (`.git` durable vs `.lodestar/` desechable) → crates separadas.
- `lodestar-core` lleva `#![forbid(unsafe_code)]`, feature `schemars` (gated, para que el MCP
  derive `JsonSchema` en los DTO) y feature `render` (pulldown-cmark para HTML de preview).

---

## 4. `lodestar-core` — modelo canónico

Módulos: `model` · `conform` · `links` · `query` · `graph` · `generate` · `export` · `diff`.
Primitivas puras como funciones libres (port 1:1 del prototipo: `split_front`, `parse_yaml`,
`dump_yaml`, `build_raw`, `parse_file`, `resolve_link`, `basename/dir_of/concept_id`); los
agregados de bundle como métodos de `Bundle`.

### 4.1 El contrato de tipos (definido UNA vez en `lodestar-core::types`)

> Esto resuelve la mayor familia de contradicciones del workflow: cada capa había redeclarado
> estos tipos con nombres y orden distintos. Se congela aquí; todas las fachadas hacen `use` de ellos.

```rust
/// Ruta relativa al root del bundle. Newtype VALIDADO: ::new rechaza absolutas, `..`,
/// y normaliza. Es el ÚNICO chokepoint de path-traversal para create/update. Prohibido
/// `type RelPath = String`.
pub struct RelPath(String);
impl RelPath { pub fn new(s: &str) -> Result<Self, CoreError> { /* reject .. / abs */ } }

/// Orden DELIBERADO: Err es el máximo, así `checks.iter().map(|c| c.level).max()` = peor.
/// Serializa en minúsculas: "err"|"warn"|"info"|"pass".
#[derive(PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity { Pass, Info, Warn, Err }

/// 15 códigos OKF. UNA sola enum (no CheckCode/RuleCode/ConfCode). El valor de wire ES
/// la cadena con guion gracias al rename por variante.
#[derive(Serialize, Deserialize)]
pub enum CheckCode {
  #[serde(rename = "OKF-FM01")] OkfFm01,   // falta frontmatter
  #[serde(rename = "OKF-FM02")] OkfFm02,   // frontmatter sin cerrar
  #[serde(rename = "OKF-FM03")] OkfFm03,   // YAML malformado
  #[serde(rename = "OKF-TYPE")] OkfType,   // falta `type`  (única regla dura)
  #[serde(rename = "REC-TITLE")] RecTitle, #[serde(rename = "REC-DESC")] RecDesc,
  #[serde(rename = "FMT-TAGS")] FmtTags,   #[serde(rename = "FMT-TS")] FmtTs,
  #[serde(rename = "LINK-STUB")] LinkStub, #[serde(rename = "LINK-REL")] LinkRel,
  #[serde(rename = "ORPHAN")] Orphan,      #[serde(rename = "BODY-STRUCT")] BodyStruct,
  #[serde(rename = "OKF-IDX")] OkfIdx,     #[serde(rename = "OKF-LOG")] OkfLog,
  #[serde(rename = "OKF-CONFLICT")] OkfConflict,  // marcadores de merge sin resolver (hard-fail)
}

/// Campos = los del prototipo `chk(level, code, msg, targets)`. NO `severity`/`message`.
#[derive(Serialize, Deserialize)]
pub struct Check { pub level: Severity, pub code: CheckCode, pub msg: String,
                   pub targets: Vec<RelPath> }   // targets SIEMPRE presente (array, nunca null)

/// Frontmatter: 7 KNOWN_FM tipados + extra para claves de productor. `status` ES typed
/// (es la 7ª KNOWN_FM y dirige el ciclo draft|review|accepted|deprecated; su orden importa
/// para build_raw). tags/timestamp se guardan RAW (serde_yaml::Value) para poder detectar
/// FMT-TAGS (no-lista) y FMT-TS (no-ISO) sin perder la malformación.
pub struct Frontmatter {
  pub r#type: Option<String>, pub title: Option<String>, pub description: Option<String>,
  pub resource: Option<String>, pub tags: Option<serde_yaml::Value>,
  pub timestamp: Option<serde_yaml::Value>, pub status: Option<String>,
  #[serde(flatten)] pub extra: BTreeMap<String, serde_yaml::Value>,
}

/// parse_file NUNCA devuelve Err por contenido: FM01/02/03 son Checks (datos), no Results.
pub struct ParsedFile { pub kind: FileKind, pub fm: Option<Frontmatter>,
                        pub fm_err: Option<FmError>, pub body: String, pub raw: String }
pub enum FileKind { Concept, Index, Log }          // reserved = kind != Concept
pub enum FmError { Missing, Unclosed, Malformed(String) }

/// El resultado de analyze(). Nombres = los del prototipo y los consumidores (inn, perFile,
/// out de strings). camelCase en wire. Rich link-metadata (href, relativo) va aparte, NO en `out`.
#[serde(rename_all = "camelCase")]
pub struct Analysis {
  pub concepts: Vec<RelPath>,
  pub out: BTreeMap<RelPath, Vec<RelPath>>,        // adyacencia de strings
  pub inn: BTreeMap<RelPath, Vec<RelPath>>,        // backlinks
  pub in_index: BTreeSet<RelPath>,
  pub dangling: Vec<RelPath>, pub orphans: Vec<RelPath>,
  pub per_file: BTreeMap<RelPath, Vec<Check>>,
  pub hard_fail: usize, pub warn_count: usize,     // hard_fail = #ficheros con algún Err
}

#[serde(rename_all = "camelCase")]
pub struct GraphModel { pub nodes: Vec<GraphNode>, pub edges: Vec<Edge> }   // `edges`, no `links`
pub struct GraphNode { pub id: RelPath, pub ghost: bool, pub r#type: Option<String>,
                       pub status: Option<String> }
pub struct Edge { pub source: RelPath, pub target: RelPath, pub dangling: bool }
```

### 4.2 Superficie pública de `Bundle`

```rust
impl Bundle {
  // construcción
  pub fn from_files(files: BTreeMap<RelPath, String>) -> Self;
  pub fn analyze(&self) -> &Analysis;                       // cacheado (OnceCell)
  // lectura semántica (todas determinísticas desde ficheros)
  pub fn list_concepts(&self) -> Vec<ConceptSummary>;       // tree rows + orphan/invalid flags
  pub fn backlinks(&self, p: &RelPath) -> Backlinks;        // inbound(LinkRef)+index_refs+out+dangling
  pub fn neighborhood(&self, p: &RelPath, depth: u32, dir: Direction) -> Neighborhood;
  pub fn graph_model(&self) -> GraphModel;
  pub fn query(&self, dsl: &str) -> Vec<QueryHit>;          // un solo tokenizer (§4.3)
  pub fn validate_draft(&self, fm: &Frontmatter, body: &str) -> Vec<Check>; // contenido SIN guardar
  // escritura validada (OKF logic, NO en la fachada)
  pub fn create_concept(&self, p: &RelPath, ty: &str, /*…*/) -> WriteOutcome;
  pub fn merge_frontmatter(&self, p: &RelPath, patch: FrontmatterPatch) -> WriteOutcome; // null borra
  // generación PURA: devuelve un plan; la workspace lo aplica
  pub fn gen_index(&self, dir: &str) -> Mutation;
  pub fn gen_tag_indexes(&self) -> Mutation;                // purga tags obsoletos
  pub fn export_zip<W: Write + Seek>(&self, w: W) -> Result<(), CoreError>;
}

pub enum Direction { Out, In, Both }   // Out=dependencias · In=blast-radius/impacto · Both=mapa local
pub struct WriteOutcome { pub path: RelPath, pub raw: String, pub hash: [u8;32], pub written: bool,
                          pub rejected: Option<String>, pub checks: Vec<Check>, pub bundle_hard_fail: usize }
pub struct Mutation { pub writes: BTreeMap<RelPath, String>, pub deletes: Vec<RelPath> }
```

- **Escrituras validadas** rechazan por defecto un fichero que introduciría un `Err`
  (la regla dura: `type` no vacío). Rechazo = `Ok(written:false, rejected:<motivo>)`, **no** un `Err`,
  para que MCP/GUI reciban feedback accionable. Flag `allow_nonconformant` para forzar.
- **Generadores puros**: devuelven `Mutation`; la **workspace** la aplica por el único camino de
  escritura y calcula `{written, removed, unchanged}` diffeando contra disco (de ahí sale el `--check` de CI).

### 4.3 Query (un solo tokenizer, semántica de subcadena)

Un único `tokenize_query` + `match_token` en el core (port de `tokenizeQuery/matchToken/isPredicate`).
Soporta `field:val` (subcadena), `field=val` (exacto), `-neg`, `has:`/`no:`, `is:orphan|invalid|reserved|linked`,
`body:`, texto suelto. Conserva el quirk de **gating de fichero reservado ANTES de negar**.
`body:`/texto suelto son **subcadena** (no token FTS) para paridad con el prototipo. FTS5 se usa
solo como acelerador/superset, **nunca** como único pre-filtro de subcadena (perdería matches reales).

### 4.4 Tipos de versionado (git) — también en `lodestar-core::types`

> El workflow de git triplicó estos tipos (`CommitMeta`/`HistoryEntry`/`VcsCommit`, `OkfDiff` ×2,
> cache de conformidad keyed de 3 formas). Se congelan aquí en **una** familia, como el resto del contrato.

```rust
/// SHA de commit. Newtype validado (como RelPath), sin I/O. git2::Oid NUNCA cruza la frontera de vcs.
pub struct Sha(String);

/// Una fila del historial. UNA sola definición. time en SEGUNDOS unix (como git), autor estructurado.
#[serde(rename_all = "camelCase")]
pub struct CommitRow { pub id: Sha, pub short: String, pub message: String, pub author: Author,
  pub time_unix: i64, pub parents: Vec<Sha>, pub conformance: Option<CommitConformance> }
pub struct Author { pub name: String, pub email: String }

/// Conformidad de un commit = proyección de Analysis sobre su árbol. Cacheada CRUDA (sin strictness);
/// el veredicto del gate (¿warns bloquean?) se deriva AL LEER de lodestar.toml.
#[serde(rename_all = "camelCase")]
pub struct CommitConformance { pub hard_fail: usize, pub warn_count: usize, pub conform: bool }

/// El diff semántico OKF (port de diffSnap). UNA sola familia, camelCase wire. NO es el diff de texto de git.
#[serde(rename_all = "camelCase")]
pub struct OkfDiff { pub files: Vec<FileDiff>, pub generated: Vec<GeneratedChange>,
  pub stats: DiffStats, pub status_changes: Vec<StatusChange>, pub suggested: MessageHint }
pub struct FileDiff { pub path: RelPath, pub kind: ChangeKind,           // Add|Mod|Remove
  pub fm: Vec<FieldChange>, pub body: Vec<BodyHunk>,
  pub links_added: Vec<RelPath>, pub links_removed: Vec<RelPath> }
pub struct FieldChange { pub key: String, pub from: Option<String>, pub to: Option<String> } // orden status-first
pub enum BodyHunk { Context(String), Add(String), Remove(String), Gap(u32) }   // LCS + plegado de contexto
pub struct StatusChange { pub path: RelPath, pub from: Option<String>, pub to: Option<String> }
pub struct DiffStats { pub added: usize, pub modified: usize, pub removed: usize }
pub enum MessageHint { AddSingle{title:String}, StatusSingle{to:String,title:String},
                       Update{added:usize,modified:usize,removed:usize} }  // i18n via catálogo en la fachada

/// Estado del repo — detecta merge/rebase en curso (bloquea "guardar versión").
pub enum RepoState { Clean, Merging, Rebasing, CherryPicking, Reverting }
```

---

## 5. `lodestar-store` — SQLite/FTS5 + watcher

Dueño **único** del DDL en `<bundle>/.lodestar/index.db` (gitignored, WAL, siempre reconstruible).

- **Materializa**: `files` (frontmatter promovido a columnas + `frontmatter_json`), `links`
  (con flag `src_is_index` → `in_index` se deriva de ahí; una sola tabla), `tags`,
  `diagnostics` (solo checks **locales**), FTS5 externo sobre `(title, description, body)`.
- **Sintetiza on-demand** (no materializa, evita invalidación en cascada): backlinks
  (índice sobre `links.dst`), orphans/ghosts (vistas), `LINK-STUB`/`ORPHAN`, neighborhood y
  **blast-radius direccional** (CTE recursivo sobre aristas inversas — distinto del neighborhood no dirigido).
- **Cold rebuild**: `ignore::WalkBuilder` → `core::parse_file` → upsert en una transacción.
- **Incremental**: `notify-debouncer-full` (~250 ms) → gate por mtime+size y **hash blake3** de
  contenido (descarta no-ops y los echoes de nuestras propias escrituras) → upsert/delete + recompute
  del vecindario afectado. `reconcile_all()` repara drift tras tormentas de eventos.
- **Bus de eventos**: `crossbeam` `IndexEvent` (síncrono, runtime-neutral). El MCP lo puentea a
  tokio; Tauri a `app.emit`; la CLI lo ignora.

El test de paridad obligatorio: `hard_fail`/backlinks/orphans/dangling vía SQL == vía `core::analyze`
sobre la misma fixture. Si difieren, es bug de la cache.

---

## 6. `lodestar-workspace` — el handle unificado

Compone `lodestar-core` (puro) + `lodestar-store`. Es lo que ven las fachadas. Reglas:

- **Un solo watcher por proceso** que posee el **único escritor** de SQLite. Los comandos
  **nunca** escriben la cache directamente: escriben el `.md` (atómico temp+rename) y dejan que el
  watcher reconcilie. Esto elimina la carrera de doble-escritor.
- **Echo-suppression** = el hash blake3 de la cache es la única autoridad; el `hash` se expone en
  cada DTO de lectura/escritura para que el editor de Svelte distinga su propio echo de una edición externa.
- Error unificado `WorkspaceError` que envuelve `CoreError` + `CacheError` (las fachadas mapean a su exit code / toast).

```rust
impl Workspace {
  pub fn open(root: &Path) -> Result<Self, WorkspaceError>;   // abre/crea cache, arranca watcher
  pub fn open_ephemeral(root: &Path) -> Result<Self, _>;      // sin cache (CLI hermético)
  pub fn subscribe(&self) -> crossbeam::Receiver<IndexEvent>;
  pub fn snapshot(&self) -> BundleSnapshot;                    // files + analysis + graph, todo junto
  // delega en core para semántica; aplica Mutations por el único camino de escritura
  pub fn backlinks/neighborhood/query/conformance/create_concept/merge_frontmatter/
         generate_index/generate_tag_indexes/export/add_log_entry(...) -> …;
}
```

---

## 7. Las tres fachadas

Cada tool de MCP y cada subcomando de CLI = un shell de 5–15 líneas: resuelve root → llama **un**
método de `Workspace` → serializa el DTO ya estructurado. **Cero lógica OKF en las fachadas.**

### 7.1 Tauri (`src-tauri`)
- 100% del acceso a disco/diálogo vive en Rust. La webview no recibe permiso `fs`/`shell`/`dialog`.
- Comandos `async` que delegan el trabajo pesado a `spawn_blocking` (los guards `RwLock`/`Mutex`
  nunca cruzan un `.await`).
- **El watcher es el único emisor de cambios.** Los comandos mutadores devuelven su propio resultado
  optimista; el evento `bundle:changed` (debounced) refresca las decoraciones globales.
- **Un solo evento de snapshot**: `bundle:changed` con `{ snapshot: BundleSnapshot, changed: string[] }`.
  Nombres de comando/evento pinned en una constante compartida; `ipc.ts` **generado** desde los tipos Rust.

Tabla de comandos (nombres congelados): `open_bundle` · `pick_dir` · `get_snapshot` ·
`list_concepts` · `read_concept` · `write_concept` (enum `Raw|Structured`) · `create_concept` ·
`delete_concept` · `merge_frontmatter` · `validate_draft` · `conformance` · `query` · `backlinks` ·
`neighborhood` · `graph_model` · `generate_index` · `generate_tags` · `add_log_entry` · `export` ·
`get_settings` · `set_setting`. Error `{code, message}` con `code` estable (`NO_BUNDLE` → onboarding).

### 7.2 MCP (`lodestar-mcp`, rmcp, stdio)
Scope = **semántica, no CRUD** (Claude Code ya tiene Read/Write/Edit). Logs solo a stderr; stdout = JSON-RPC.

- **Tools**: `find_backlinks` · `find_orphans` · `find_dangling` · `neighborhood(concept, depth, direction)` ·
  `conformance_check(path?)` · `query(dsl)` · `create_concept`(validado) · `update_frontmatter`(validado, patch con null-borra) ·
  `generate_index` · `generate_tag_indexes`.
- **Resources** (read-only): lista de concepts · índice de frontmatter · gate de conformidad en vivo · grafo de enlaces.
- No expone `read_file`/`write_file`. El valor es lo que los ficheros crudos no dan barato: backlinks resueltos,
  ghosts, huérfanos, impacto, la puerta OKF, query estructurada y **escrituras validadas**.

### 7.3 CLI (`lodestar-cli`, clap)
Subcomandos `init` · `check` · `index` · `tags` · `export` · `reindex` · `import`.
Exit codes: `0` conforme · `1` hard-fail (la puerta de CI) · `2` uso · `3` runtime/IO · `4` drift de generadores (`--check`).
`lodestar check` **reconcilia o corre efímero** antes de leer, para que una cache obsoleta nunca deje pasar el gate.
Salida humana / `--json` / SARIF.

---

## 8. Frontend (Svelte 5 + Vite)

Porta la UI del prototipo **verbatim en aspecto** (mismo `<style>`, mismas variables CSS y atributos
`data-theme/view/explorer/rail-*`) pero **invierte la propiedad de los datos**: el `files{}` y
`analyzeBundle()` del prototipo se van a Rust; la webview es una vista fina sobre un `BundleSnapshot` empujado.

- **Stores** (`svelte/store` clásicos, shapes explícitos verificables contra los tipos Rust): el snapshot
  empujado es la única fuente; `tree rows`, `conformance pill`, `backlinks`, `graph`, `perFile` son `derived`.
  Writables = `bundleRoot`, buffers de edición por path (`OpenDoc` con baseline/dirty/inflight-hash), `query`,
  y estado efímero de vista/layout/tema. Runes ($state/$derived/$effect) solo para estado local de componente.
- **Editor multi-escritor**: los pushes del snapshot nunca pisan un buffer sin guardar; la supresión de echo usa
  el `hash` que devuelve cada escritura (distingue mi propio write volviendo por el watcher de una edición externa,
  que sí levanta un banner de conflicto).
- **El grafo es una ISLA imperativa**: `createStarMap(svg)` posee el SVG, el loop rAF de física y el mapa
  persistente de posiciones (el `GPOS` del prototipo). Svelte lo monta y le pasa nodos/aristas/actual/matched por
  métodos dentro de `$effect`, **nunca** con `{#each}` reactivo — así los cambios de topología hacen diff-merge
  (preservan layout) y selección/búsqueda son repintados O(1). Para 10k nodos: **Barnes-Hut/quadtree** (la sim
  all-pairs O(n²) del prototipo no escala) + cap del scope global (clustering o por defecto "vecindad") + virtualización de filas.
- **Contrato IPC**: un `.d.ts` generado desde los tipos Rust (ts-rs/specta) que `ipc.ts` importa. Mata toda la
  deriva de nombres/casing entre Rust y TS.

---

## 9. Flujo de datos

```
            (humano en app)   (agente vía MCP)   (agente a pelo / git pull)
                    \                |                 /
                     ▼               ▼                ▼
                 escribe  un  .md  atómico  en  disco   ◄── ÚNICA fuente de verdad
                                    │
                                    ▼
                     notify watcher (1 por proceso, gate por hash blake3)
                                    │  (descarta echoes / no-ops)
                                    ▼
                 lodestar-store: upsert incremental → .lodestar/index.db
                                    │  emite IndexEvent (crossbeam)
                                    ▼
                 lodestar-workspace recomputa Analysis (core) + snapshot
                                    │
                  ┌─────────────────┼──────────────────┐
                  ▼                 ▼                  ▼
            Tauri app.emit     MCP invalida        CLI (ignora;
          bundle:changed       resources           one-shot reconcile)
          {snapshot,changed}
                  │
                  ▼
        stores Svelte → tree / pill / backlinks / star-map se re-derivan
```

**Git (versionado).** Un `commit` mueve refs pero **no cambia bytes** → el gate de hash blake3 es ciego a él:
el watcher vigila además un subconjunto de `.git` (`HEAD`, `refs/heads/`, `packed-refs`, `logs/HEAD`) y emite
`vcs:changed`, y el pill se actualiza **optimistamente** con el `Sha` que devuelve el propio commit (nunca espera
el echo) + un reconcile al enfocar la ventana. Un `restore` reescribe ficheros del working tree por el **único
escritor** (lote auto-originado que el reconcile absorbe). `commit`/`restore`/`init` son operaciones de la
**workspace** → el invariante de único escritor se preserva. Detalle completo en §13.

---

## 10. Decisiones ratificadas (resuelven las contradicciones del workflow)

| # | Tensión entre capas | Resolución |
|---|---|---|
| 1 | ¿Modelo en memoria o SQLite es la verdad computada? | **Core es la autoridad** en las 3 fachadas; SQLite = acelerador/FTS verificado por test de paridad. `lodestar check` reconcilia antes de leer. A escala, el mismo API del core se alimenta de proyecciones SQL (trait `ConceptStore`), no de todo el corpus en RAM. |
| 2 | ¿Dónde vive `Workspace`? | En **`lodestar-workspace`** (glue), no en el core. `rusqlite`/`notify` solo en `store`. Core queda puro. |
| 3 | `Check`/`Severity`/`CheckCode` triplicados con nombres y `Ord` distintos | **Una definición** en `lodestar-core::types`. `Check {level, code, msg, targets}`; `Severity` ordenada `Pass<Info<Warn<Err` (`.max()`=peor) en minúsculas; `CheckCode` con `#[serde(rename="OKF-…")]`. Borrar duplicados. |
| 4 | Bug del gate: `Severity{Err,…}` + `Ord` derivado → `.max()` da `Pass` → CI nunca falla | Orden corregido (Err máximo) **o** `hard_fail = #ficheros con algún Err` (conteo). Test: 1 Err + 1 Pass cuenta como hard_fail. |
| 5 | `Analysis` con `out:Vec<Link>` vs `Vec<RelPath>`, `inn` vs `backlinks`, `checks` vs `per_file` | Congelado: `out` = strings, `inn`, `per_file`, camelCase en wire. Metadata de link aparte. |
| 6 | ¿Capa DTO paralela (CheckDto/AnalysisDto)? | **No.** Un solo esquema de wire; el `.d.ts` se genera desde Rust. Se borra la DTO duplicada de Tauri. |
| 7 | Nombres de evento/comando divergentes (`bundle:changed` vs `bundle://changed`, `query` vs `query_bundle`…) | Registro de constantes compartido + `ipc.ts` generado + smoke test que abre bundle, edita y asserta snapshot poblado. |
| 8 | Doble escritor de SQLite (comando + watcher) | **Un watcher = único escritor.** Comandos solo escriben el `.md`. |
| 9 | `RelPath` newtype vs `type RelPath = String` | Newtype validado en todas partes (es el chokepoint de path-traversal). |
| 10 | DDL del grafo definido por dos arquitectos; `checks` vs `diagnostics` | `store` es dueño único del DDL; ORPHAN/LINK-STUB **sintetizados** (no materializados); columnas casan con los nombres del `Check`. |
| 11 | `body:` subcadena (proto) vs FTS MATCH (token) | Subcadena en todas las fachadas; FTS solo como acelerador superset. Un solo `match_token`. |
| 12 | Generadores puros vs que escriben | Puros (devuelven `Mutation`); la workspace aplica y diffea para `{written,removed,unchanged}`. |
| 13 | `merge_frontmatter` (patch, null-borra) no existía en el core | Vive en el **core** (es lógica OKF), no en el MCP. |
| 14 | Falta `schemars` para el outputSchema del MCP | Feature `schemars` en el core que gatea `#[derive(JsonSchema)]` en los DTO públicos. |
| 15 | ¿Dónde vive git? | Crate `lodestar-vcs` (git2/libgit2), hermana de `store`; core sin git. **libgit2, no shell** a `git` (no ejecutar hooks/config de bundles ajenos = RCE). |
| 16 | "Restore soft" podía **perder trabajo sin commitear** | No-destructivo *de historial* pero reescribe el working tree → **checkpoint automático** si hay cambios sin guardar; excluye `log.md` curado; **regenera** `index`/`tags` tras restaurar. |
| 17 | Marcadores de conflicto pasaban la conformidad | Nuevo check **`OKF-CONFLICT`** (hard-fail) por `<<<<<<<`/`=======`/`>>>>>>>`/`\|\|\|\|\|\|\|`; el gate y la conformidad-por-commit los detectan en las 3 fachadas. |
| 18 | Merge/rebase en curso no detectado | `RepoState` desde `repository.state()`; pill/overlay muestran "resolviendo conflicto" y `commit` se niega sobre índice no-merge. |
| 19 | Pill obsoleto tras commit (no cambia bytes) | Defensa en profundidad: ref-watch del gitdir real (incl. `logs/HEAD`) + update **optimista** con el `Sha` + reconcile al enfocar. El ref-watch es pista, no garantía. |
| 20 | Tipos commit/diff/cache triplicados | Una familia en `core::types` (§4.4); cache de conformidad por **tree-oid** (dedup de reverts) con gate `ruleset_version`; golden cross-fachada. |
| 21 | Contador "sin guardar" usaba `diffSnap` (caro/edición) | **Hash por path** contra el HEAD-map en RAM (O(cambiados)); `OkfDiff` completo perezoso solo al abrir overlay/modo Cambios; LCS con guarda + DP dos-filas/Hirschberg; saltar blobs binarios. |

---

## 11. Presupuesto de rendimiento

Objetivos explícitos (gate de bench con una fixture sintética de 10k concepts):

- **Cold open** 10k concepts < ~2 s · **edit → UI** < 150 ms · **grafo** 60 fps hasta N nodos visibles.
- A escala: servir `list`/`query`/`analysis` desde proyecciones SQL (no materializar todos los cuerpos en RAM);
  **eventos delta** en vez de full-snapshot; **Barnes-Hut** en la sim; cap/cluster del grafo global; virtualización del árbol.

---

## 12. Concerns transversales (con dueño asignado)

| Tema | Decisión |
|---|---|
| **Migración** del prototipo (datos en `localStorage`) | `lodestar import` materializa `STORE_KEY` a `.md` + cache, **`git init`, y replica `versions[]` como commits retro-fechados** (autor/fecha/mensaje de cada snapshot vía `git2::Signature`) — reproduce el historial del prototipo en vez de tirarlo. Sin esto, los early adopters pierden datos e historial. |
| **Versionado OKF** (`okf_version`) | Política para versión desconocida/futura (warn-and-degrade); `CheckCode` aditivo-solo con deprecación explícita; exponer `okf_version` en la conformidad. Distinto del `user_version` de la cache. |
| **i18n** | Mensajes de conformidad **keyed por código** (la UI localiza). Cabeceras de artefactos generados (`index.md`/tags) **fijas canónicas** como consts (los bytes generados son ficheros commiteados: cambiar locale los churnea). UI en español, strings externalizadas a catálogo. |
| **Packaging** | Tauri updater + firma/notarización (macOS/Windows); los 3 binarios desde un release etiquetado; comando de lanzamiento del MCP documentado para Claude Code; política de compat app/CLI/MCP/schema. CI de release. |
| **Testing/paridad** | Crate de fixtures; test diferencial (proto JS en node vs core); golden cross-fachada (CLI `--json` == MCP `structuredContent` == comando Tauri); property test (incremental == rebuild); tests de store Svelte; e2e smoke de Tauri. |
| **Seguridad** | **DOMPurify** en el markdown (no regex casera); escapado de expresiones FTS5; threat model de una página (webview, MCP confianza-local, zip-slip, path-traversal). |
| **Errores** | Taxonomía fatal/recuperable/transitorio + afford de recuperación; código estable cruzando `CoreError`→`AppError`/exit-code; **supervisar el watcher** (panic → restart + banner, nunca UI obsoleta en silencio). |
| **Config** | Dos niveles: app-global (tema/layout/recents) y **por-bundle** (`lodestar.toml` commiteado: strictness, write policy, locale de artefactos) para que GUI/CLI/MCP coincidan. |
| **Un bundle por proceso** | Asunción documentada + lockfile que elige un único indexador cuando GUI y MCP abren el mismo bundle. Multi-ventana/multi-bundle = no-goal v1. |
| **First-run** | `lodestar init` / "crear bundle" en GUI: scaffold de `index.md` raíz con `okf_version`, `.gitignore` (incluye `.lodestar/`), **`git init` + commit inicial**. En cada `open` de un repo existente se verifica que `.lodestar/` está ignorado (idempotente; oferta "dejar de trackear" si ya estaba trackeada). |
| **Sincronización / remoto** | **No-goal v1.** `push`/`pull`/`fetch`/`clone` = responsabilidad del usuario vía su `git` CLI; lodestar lee el estado post-pull por el ref-watch pero nunca habla con remotos (libgit2 sin red). Resuelve la tensión hooks-RCE-vs-push. |
| **Paridad con `git` CLI** | libgit2 **no** corre hooks, **no** firma (`commit.gpgsign` ignorado), **no** aplica filtros LFS/`.gitattributes`. v1: commits **sin firmar**; si el repo exige firma/LFS, **avisar** en el diálogo de commit y delegar en el CLI. Nunca shell-out a `git`. |
| **Identidad / atribución** | Autor+committer separados; override (`lodestar.toml [identity]`)→git config→fallback marcado. Commits del **agente** (MCP) llevan trailer `Co-Authored-By` distinguible para que `git log`/blame no mientan. `[identity]` se añade al schema de `lodestar.toml`. |
| **CRDT (futuro)** | Documentar que la canonicalización de `build_raw` + LWW por fichero sesga contra un CRDT por-bloque. Mantener el core sin I/O para que un futuro server `axum` reuse la superficie de análisis. |

---

## 13. Versionado (git) — integración de primera clase

El prototipo añadió **"Versiones / historial"**: git escondido tras el vocabulario de lodestar. Se integra
sin romper ninguna decisión ratificada (core puro, único escritor, snapshot-push, un contrato de tipos).

### 13.1 Vocabulario

| lodestar | git |
|---|---|
| "versión" / "guardar versión" (Ctrl/Cmd+S) | commit |
| "N sin guardar" / "Al día" | working tree dirty vs HEAD (menos generados) |
| "línea principal" | la rama actual (resuelta de HEAD; no hardcodeada) |
| "restaurar versión" | materializar el árbol de un commit al working tree (soft) |
| "última versión conforme" | último commit cuyo árbol pasa la puerta OKF |
| "propuesta en revisión" | concept con `status: review` (no una rama, v1) |

### 13.2 `lodestar-vcs` (git2/libgit2)

Dueño único de git, hermana de `store`. Encapsula git2: expone `Sha`, nunca `git2::Oid`. **libgit2, no shell
a `git`** — abrir un bundle ajeno no debe ejecutar sus hooks/aliases/`include.path`/fsmonitor (RCE).
`git2::Repository` es `!Sync` → vive tras el único escritor (`Mutex<Vcs>`).

```rust
impl Vcs {
  pub fn discover(root: &Path) -> Result<Option<Vcs>>;     // None = sin .git (modo "activar versiones")
  pub fn init(root: &Path) -> Result<Vcs>;                 // git init + .gitignore + commit inicial
  pub fn status(&self) -> RepoStatus;                      // dirty set + RepoState (merge en curso)
  pub fn log(&self, limit: usize) -> Vec<CommitRow>;       // metadatos baratos (revwalk), sin leer árboles
  pub fn log_for_path(&self, p: &RelPath, limit: usize) -> Vec<CommitRow>;  // con techo de commits escaneados
  pub fn tree_files(&self, sha: &Sha) -> Result<FileMap>;  // árbol de un commit → file-map SIN tocar el working tree
  pub fn commit(&self, msg: &str, author: &Author) -> Result<Sha>;          // stage + commit del working tree
  pub fn current_branch(&self) -> Option<String>;          // la "línea"; HEAD desacoplado = None
  // restore NO lo hace vcs: la workspace computa un core::Mutation y lo aplica por el único escritor
}
```

- **Nunca escribe el working tree.** `restore` devuelve un `core::Mutation` que aplica la **workspace** por el
  único escritor (igual que los generadores) → preserva el invariante de único escritor. Blobs binarios/no-UTF8 se
  **saltan y diagnostican** en `tree_files` (no abortan el árbol ni la cache de conformidad).
- **Degradación sin `.git`**: `discover` con techo en el root del bundle (no engancha un repo ancestro como
  `~/.git`); tres estados distintos: sin-repo ("activar versiones"→`init`), repo-vacío, con-historial.

### 13.3 Diff semántico OKF (puro, en el core)

El módulo `core::diff` (port de `diffSnap/fmDiff/lineDiff/collapseDiff`) es la **única verdad computada** del diff;
lo renderizan igual las fachadas y el frontend (`OkfDiff`, §4.4). vcs da dos file-maps (árbol vs árbol, o HEAD vs
working); el core da el *significado*: frontmatter por-campo (orden `status` primero), cuerpo por LCS con plegado de
contexto, transiciones de ciclo de vida, impacto en el grafo de enlaces, y **segregación de generados** (index/log/
tags no cuentan como edición manual).

- **Rendimiento**: el contador "sin guardar" es comparación de **hash por path** contra el HEAD-map en RAM
  (O(cambiados)), **nunca** `diffSnap`. El `OkfDiff` completo (con LCS) se computa **perezoso**, solo para el
  fichero abierto. El LCS lleva guarda de tamaño (fallback grueso por umbral) y DP en dos filas + Hirschberg
  (mata el muro de memoria O(n·m): un fichero de 10k líneas reservaba ~400 MB).

### 13.4 Conformidad por commit (la pieza estrella)

Cada versión guarda su conformidad — el `confOf(snap)` del prototipo hecho real:
`core::Bundle::from_files(vcs.tree_files(sha)).analyze()` → `CommitConformance{hardFail,warnCount,conform}`.

- **Cache** en `.lodestar/index.db` keyed por **tree-oid** (content-addressed: dedup de reverts/cherry-picks),
  gated por `ruleset_version` (**hash de las definiciones de reglas** — imposible cambiar un check sin invalidar la
  cache). El árbol es inmutable → la fila nunca se invalida por edición.
- **Perezosa y acotada**: solo HEAD (el gate/pill), los commits visibles de la página del timeline (rellenados
  off-thread, punto "computando…", persistidos), un commit abierto, o el barrido early-exit de `last_conforming()`.
  Nunca se analiza todo el DAG al abrir.
- **Incremental**: reusa los checks locales por-fichero del commit padre para blobs con oid sin cambios; solo
  recomputa el pase global del grafo. O(M×cambiados + grafo) en vez de O(M×árbol completo).
- Se cachea **crudo** (`hardFail`,`warnCount`); el veredicto del gate (¿warns bloquean?) se deriva **al leer** de
  la strictness de `lodestar.toml` — la strictness nunca se hornea en la cache.

### 13.5 La puerta OKF ↔ git

`lodestar check [--staged | --rev SHA | --range a..b]` significa "¿es conforme este árbol?".
- **pre-commit** → `lodestar check --staged` (juzga el índice staged, no el working sucio); **pre-push** →
  `--rev HEAD`; CI corre el mismo binario. `lodestar hooks install` los cablea.
- **Commits de la app van por libgit2 → no disparan hooks.** Por eso la **workspace corre `check` ella misma antes
  de `commit`**; los hooks instalados solo cubren commits hechos por el `git` CLI / CI. Documentado para no engañar.

### 13.6 Las cuatro correcciones de seguridad (ship-blockers)

1. **Restore no pierde trabajo.** No-destructivo *de historial* pero **reescribe el working tree**: si hay cambios
   sin guardar, primero hace un **commit de checkpoint** automático (trabajo perdido → "una versión más a la que
   volver"); excluye el `log.md` curado; **regenera** `index`/`tags` tras restaurar.
2. **`OKF-CONFLICT`** (hard-fail): marcadores `<<<<<<<`/`=======`/`>>>>>>>`/`|||||||` en cuerpo o frontmatter —
   antes pasaban la conformidad en silencio tras un `git pull` conflictivo.
3. **`RepoState`** desde `repository.state()`: merge/rebase en curso bloquea "guardar versión" y avisa
   "resolviendo conflicto" (en vez de `add_all`+commit sobre índice no-merge → basura).
4. **Pill nunca obsoleto.** Un commit no cambia bytes → defensa en profundidad: ref-watch del gitdir real (incl.
   `logs/HEAD`, maneja `.git`-como-fichero), update **optimista** con el `Sha`, y reconcile al enfocar.

### 13.7 Fachadas y frontend

- **Tauri**: `vcs_status` · `vcs_log` · `vcs_log_for_path` · `vcs_diff(a,b,filter?)` · `vcs_diff_working` ·
  `vcs_commit(msg, alsoLog)` · `vcs_restore(sha)` · `vcs_last_conforming` · `vcs_init`. `bundle:changed` crece un
  campo `vcs` barato (head/pendingCount/clean); el `OkfDiff`/log pesados se piden al abrir.
- **MCP**: `history(concept?)` · `diff(revA,revB)` · `last_conforming_version` · `when_changed(concept)` ·
  **`commit(message)`** (única escritura: el agente hace checkpoint y recibe la conformidad post-commit → aprende
  "no conforme" y se autocorrige). Commits del agente con trailer `Co-Authored-By`.
- **CLI**: `log` · `diff` · `last-conforming` · `hooks install` (+ `check --staged/--rev/--range`).
- **Frontend**: el **pill** de versión + popover (pendientes, recientes, "restaurar última conforme"); el **overlay**
  (timeline de la rama con puntos de conformidad por commit renderizados progresivamente + "Propuestas en revisión"
  = `status:review` + panel de diff + restaurar/comparar/filtrar a una página); y el **4º modo de editor "Cambios"**
  (diff de la página vs HEAD, `OkfDiff` perezoso). El grafo/física no se toca; un store `vcs` se alimenta de
  `vcs:changed` + el resumen barato.
- **Dos historiales distintos**: `git log` (completo, máquina, dirige el timeline) y `log.md` (changelog OKF curado
  en el bundle, validado por OKF-LOG, anexado solo si el opt-in del diálogo está activo y viaja en el mismo commit).
  No se auto-sincronizan.

### 13.8 Scope ratificado de git (v1)

| Tema | Decisión v1 |
|---|---|
| Sincronización / remoto | **No-goal.** push/pull/fetch/clone = `git` CLI del usuario; lodestar lee el estado post-pull por el ref-watch, sin red. |
| Firma de commits | **Sin firmar** (libgit2 ignora `commit.gpgsign`); si el repo la exige, avisar y delegar en el CLI. |
| LFS / `.gitattributes` | libgit2 no aplica filtros; detectar y **avisar** (no commitear un blob LFS crudo); binarios fuera de scope, se saltan. |
| Ramas | Read-only (vocabulario "línea"); no crear/cambiar/mergear desde la app. |
| Propuestas | `status: review` en la línea principal, **no** ramas/PR. Aceptar/rechazar = editar frontmatter. |
| Tags · submódulos · worktrees · repos bare | Diferidos / no soportados v1 (degradan, no crashean). |

---

## 14. Plan de construcción por fases

Cada fase se valida con el arnés de paridad antes de la siguiente.

1. **`lodestar-core` puro** + el contrato de tipos (§4) + **el diff semántico OKF** + arnés diferencial contra el
   prototipo JS. Sale aquí toda la lógica OKF; testeable sin GUI/DB.
2. **`lodestar-cli`** mínimo (`check`/`index`/`tags`/`export`) sobre el core efímero. Ya es útil como gate de CI.
3. **`lodestar-store`** (SQLite/FTS5 + watcher) + test de paridad SQL==core + property test incremental==rebuild.
4. **`lodestar-vcs`** (git2: status/log/diff/commit/restore/init + ref-watch) + cache de conformidad por commit
   (tree-oid) + `lodestar check --staged/--rev` y `hooks install`.
5. **`lodestar-workspace`** (handle unificado + bus de eventos + único escritor; compone core+store+vcs; restore con checkpoint).
6. **`src-tauri`** + **frontend Svelte** portando el prototipo verbatim (incl. pill/overlay/modo "Cambios"); `.d.ts` generado; editor multi-escritor.
7. **`lodestar-mcp`** (casi gratis: 4ª fachada sobre la misma workspace, con `commit` para agentes) + golden cross-fachada.
8. Transversales de producto: migración (con replay de historial a git), packaging/updater, i18n, seguridad, config por-bundle, first-run.
