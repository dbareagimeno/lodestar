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
| Versionado | **git** (libgit2 local + binario `git` solo para red) | Commits, ramas, historial, diff OKF, conformidad-por-commit, push/pull; vocabulario git directo; local-first |
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
        ▲          ▲                 # vcs:   git2/libgit2 (local: status/log/diff/commit/branch/merge/restore/init,
        │          │                 #        lee árboles a file-maps, ref-watch) + binario `git` confinado a la red
        │          │                 #        (push/pull/fetch). NO toca el working tree.
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
  el diff *semántico* OKF sí es lógica pura del core. **Transporte híbrido**: libgit2 para todas las
  operaciones *locales* (commit/log/diff/branch/merge/restore) — abrir o indexar un bundle ajeno **nunca**
  ejecuta sus hooks/aliases/`include.path` (la garantía RCE-safe). El binario `git` se invoca **solo** para
  las operaciones de *red* (push/pull/fetch), iniciadas explícitamente por el usuario sobre su propio repo,
  para heredar su auth (SSH-agent/credential-helpers/tokens) sin reimplementarla. El shell-out se confina a
  `vcs` y nunca corre en open/index. Git history y la cache SQLite tienen ciclos de vida opuestos (`.git`
  durable vs `.lodestar/` desechable) → crates separadas.
- `lodestar-core` lleva `#![forbid(unsafe_code)]`, feature `schemars` (gated, para que el MCP
  derive `JsonSchema` en los DTO) y feature `render` (pulldown-cmark para HTML de preview).

---

## 4. `lodestar-core` — modelo canónico

> **Superada por §20 en cuanto al MODELO DOCUMENTAL** (migración a workspaces Markdown universales,
> `docs/REFACTOR_PHASE_2.md`). Lo que sigue describe el modelo **OKF** (frontmatter de 7 campos
> tipados, `FileKind::Index`/`Log`, códigos `OKF-*`, `in_index`, generadores de índices): se conserva
> como referencia histórica de v0.2.x, **no** como comportamiento de v0.3+. Lo que §20 **no** toca de
> esta sección sigue vigente: `RelPath` como newtype validado (§4.1), la disciplina de "una sola
> definición de tipos", la pureza del core y la forma de `Check`/`Severity` (cambian los *códigos*,
> no la estructura).

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
  pub okf_version: Option<String>,                 // del index.md raíz; None si falta. §12 lo expone en la conformidad
}

#[serde(rename_all = "camelCase")]
pub struct GraphModel { pub nodes: Vec<GraphNode>, pub edges: Vec<Edge> }   // `edges`, no `links`
pub struct GraphNode { pub id: RelPath, pub ghost: bool, pub r#type: Option<String>,
                       pub status: Option<String> }
pub struct Edge { pub source: RelPath, pub target: RelPath, pub dangling: bool }

// --- DTOs de lectura de Bundle (§4.2). Se congelan AQUÍ, en core::types, como el resto (principio #4):
//     una sola definición, wire camelCase, sin capa DTO paralela. Contenido = port 1:1 del prototipo.

/// Mapa de ficheros del bundle. Es lo que come `Bundle::from_files` y lo que devuelve `vcs.tree_files`.
pub type FileMap = BTreeMap<RelPath, String>;

/// Fila del árbol de concepts (port de fileRow/renderTree). La jerarquía la deriva el front del `path`.
#[serde(rename_all = "camelCase")]
pub struct ConceptSummary { pub path: RelPath, pub title: String, pub r#type: Option<String>,
  pub status: Option<String>, pub orphan: bool, pub invalid: bool }   // title = ya resuelto (fm.title o del path); invalid = algún Check level=Err

/// Un extremo de un enlace + el href crudo tal como aparece en el `.md` (port de resolveLink).
/// Es la rich link-metadata que §4.1 dejaba "aparte". (ghost no va aquí: es de GraphNode.)
pub struct LinkRef { pub path: RelPath, pub href: String }

/// Vecindad de enlaces de un concept (port del panel de backlinks). wire camelCase.
#[serde(rename_all = "camelCase")]
pub struct Backlinks { pub inbound: Vec<LinkRef>,   // quién enlaza aquí (con el href usado)
  pub index_refs: Vec<RelPath>,                     // index.md que lo listan
  pub out: Vec<RelPath>,                            // destinos salientes resueltos
  pub dangling: Vec<String> }                       // hrefs salientes que no resuelven a ningún fichero

/// Subgrafo dirigido alrededor de un concept (reusa la forma de GraphModel; `root` = el centro).
#[serde(rename_all = "camelCase")]
pub struct Neighborhood { pub root: RelPath, pub nodes: Vec<GraphNode>, pub edges: Vec<Edge> }

/// Patch de frontmatter (merge_frontmatter / MCP update_frontmatter). Semántica merge-patch (RFC 7386):
/// clave→Some(v) escribe/reemplaza; clave→None BORRA; clave AUSENTE del mapa = no se toca. El tercer
/// estado se modela con la pertenencia al mapa (evita el Option<Option<_>> y su trampa en serde).
pub struct FrontmatterPatch(pub BTreeMap<String, Option<serde_yaml::Value>>);
```

### 4.2 Superficie pública de `Bundle`

```rust
impl Bundle {
  // construcción
  pub fn from_files(files: FileMap) -> Self;                // FileMap = BTreeMap<RelPath, String> (§4.1)
  pub fn analyze(&self) -> &Analysis;                       // cacheado (OnceCell)
  // lectura semántica (todas determinísticas desde ficheros)
  pub fn list_concepts(&self) -> Vec<ConceptSummary>;       // tree rows + orphan/invalid flags
  pub fn backlinks(&self, p: &RelPath) -> Backlinks;        // inbound(LinkRef)+index_refs+out+dangling
  pub fn neighborhood(&self, p: &RelPath, depth: u32, dir: Direction) -> Neighborhood;
  pub fn graph_model(&self) -> GraphModel;
  pub fn query(&self, dsl: &str) -> Vec<RelPath>;           // filtro de paths (port fiel: el prototipo no enriquece); tokenizer §4.3
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
Soporta `field:val` (subcadena), `field=val` (exacto), `-neg`, `has:`/`no:`,
`is:orphan|invalid|reserved|linked|accepted|draft|review|deprecated` (los cuatro últimos = predicados de `status`),
`body:`, texto suelto, y el **flip de negación** `!val` (un `!` al inicio del valor invierte `-neg`, doble-negable).
Conserva el quirk de **gating de fichero reservado ANTES de negar**. El nombre de campo es ASCII `[\w\-]+` (una
clave con acento cae a texto suelto, como en el prototipo); el valor se compara case-insensitive.
`body:`/texto suelto son **subcadena** (no token FTS) para paridad con el prototipo. FTS5 se usa
solo como acelerador/superset, **nunca** como único pre-filtro de subcadena (perdería matches reales).
`query()` devuelve **paths** (filtro, port fiel); enriquecer el hit (snippet/score vía FTS5) queda como
ampliación futura **aditiva**, fuera de la paridad v1.

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

/// Estado del repo — detecta merge/rebase en curso (bloquea el commit hasta resolver).
pub enum RepoState { Clean, Merging, Rebasing, CherryPicking, Reverting }

/// Una rama. UNA definición. `upstream` = rama remota de seguimiento (p.ej. "origin/main"), si la hay.
#[serde(rename_all = "camelCase")]
pub struct Branch { pub name: String, pub is_head: bool, pub upstream: Option<String>,
                    pub ahead: usize, pub behind: usize }   // ahead/behind vs upstream (0/0 si no hay)

/// Resultado de una operación de red (push/pull) — vía binario `git`. Sin tipos de git2::Remote.
#[serde(rename_all = "camelCase")]
pub struct SyncOutcome { pub kind: SyncKind, pub ok: bool, pub summary: String }
// pull es --ff-only (nunca conflicta in-app); push rechazado (non-ff) → ok:false + summary. Los conflictos viven
// en el `merge` local (marcadores inline → OKF-CONFLICT), no aquí.
pub enum SyncKind { Push, Pull }
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

Compone `lodestar-core` (puro) + `lodestar-store` + `lodestar-vcs`. Es lo que ven las fachadas. Reglas:

- **Un solo watcher por proceso** que posee el **único escritor** de SQLite. Los comandos
  **nunca** escriben la cache directamente: escriben el `.md` (atómico temp+rename) y dejan que el
  watcher reconcilie. Esto elimina la carrera de doble-escritor.
- **Echo-suppression** = el hash blake3 de la cache es la única autoridad; el `hash` se expone en
  cada DTO de lectura/escritura para que el editor de Svelte distinga su propio echo de una edición externa.
- Error unificado `WorkspaceError` que envuelve `CoreError` + `CacheError` (las fachadas mapean a su exit code / toast).

```rust
impl Workspace {
  pub fn open(root: &Path) -> Result<Self, WorkspaceError>;   // abre/crea cache, arranca watcher, Vcs::discover
  pub fn open_ephemeral(root: &Path) -> Result<Self, _>;      // sin cache (CLI hermético)
  pub fn subscribe(&self) -> crossbeam::Receiver<IndexEvent>;
  pub fn snapshot(&self) -> BundleSnapshot;                    // files + analysis + graph, todo junto
  // delega en core para semántica; aplica Mutations por el único camino de escritura
  pub fn backlinks/neighborhood/query/conformance/create_concept/merge_frontmatter/
         generate_index/generate_tag_indexes/export/add_log_entry(...) -> …;
  // git (vía lodestar-vcs): commit/restore/switch_branch/merge convierten el file-map de vcs en core::Mutation y
  //   lo aplican por el ÚNICO escritor (+ checkpoint si hay cambios sin commitear, §13.6); create_branch/branches/
  //   vcs_log/vcs_diff/last_conforming son lecturas; pull/push delegan en el subproceso `git` (escritor externo).
  pub fn commit/restore/switch_branch/merge/create_branch/branches/vcs_log/vcs_diff/pull/push/last_conforming(...) -> …;
}
```

---

## 7. Las tres fachadas

Cada tool de MCP y cada subcomando de CLI = un shell de 5–15 líneas: resuelve root → llama **un**
método de `Workspace` → serializa el DTO ya estructurado. **Cero lógica OKF en las fachadas.**

### 7.1 Tauri (`src-tauri`)

> **Retirado de `main`** (giro headless, §19.1): la fachada Tauri se movió a la rama
> `experimental/ui-desktop`. Sección conservada como diseño ratificado de referencia.

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
`get_settings` · `set_setting` (+ comandos `vcs_*` de versionado en §13.7). Error `{code, message}` con `code` estable (`NO_BUNDLE` → onboarding).

### 7.2 MCP (`lodestar-mcp`, rmcp, stdio)
Scope = **semántica, no CRUD** (Claude Code ya tiene Read/Write/Edit). Logs solo a stderr; stdout = JSON-RPC.

- **Tools**: `find_backlinks` · `find_orphans` · `find_dangling` · `neighborhood(concept, depth, direction)` ·
  `conformance_check(path?)` · `query(dsl)` · `create_concept`(validado) · `update_frontmatter`(validado, patch con null-borra) ·
  `generate_index` · `generate_tag_indexes`.
- **Resources** (read-only): lista de concepts · índice de frontmatter · gate de conformidad en vivo · grafo de enlaces.
- No expone `read_file`/`write_file`. El valor es lo que los ficheros crudos no dan barato: backlinks resueltos,
  ghosts, huérfanos, impacto, la puerta OKF, query estructurada y **escrituras validadas**.

### 7.3 CLI (`lodestar-cli`, clap)
Subcomandos `init` · `check` · `index` · `tags` · `export` · `reindex` · `import` (+ los subcomandos git de §13.7:
`log` · `diff` · `last-conforming` · `branch` · `merge` · `pull` · `push` · `hooks install`).
Exit codes: `0` conforme · `1` hard-fail (la puerta de CI) · `2` uso · `3` runtime/IO · `4` drift de generadores (`--check`).
`lodestar check` **reconcilia o corre efímero** antes de leer, para que una cache obsoleta nunca deje pasar el gate.
Salida humana / `--json` / SARIF.

---

## 8. Frontend (Svelte 5 + Vite)

> **Retirado de `main`** (giro headless, §19.1): la UI de escritorio (`frontend/` + `src-tauri/`) se
> movió íntegra a la rama `experimental/ui-desktop`. Esta sección se conserva como **diseño
> ratificado de referencia** (histórico), no como parte del motor headless; si la UI vuelve a
> evolucionar, se hace en esa rama.

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
el echo) + un reconcile al enfocar la ventana. Un `restore`, un cambio de rama (`switch_branch`) y un `merge`
reescriben ficheros del working tree por el **único escritor** (lote auto-originado que el reconcile absorbe). Un
`pull` (vía binario `git`) cambia **bytes y refs** a la vez → lo absorben el watcher (bytes) y el ref-watch (refs)
como cualquier escritor externo. `commit`/`restore`/`switch_branch`/`merge`/`init` son operaciones de la
**workspace** → el invariante de único escritor se preserva. Detalle completo en §13.

---

## 10. Decisiones ratificadas (resuelven las contradicciones del workflow)

> **Nota (workspaces Markdown universales §20, 2026-07-23).** Las filas que fijan **disciplina de
> arquitectura** siguen vigentes sin cambios: **#1** (el core es la autoridad, SQLite acelerador),
> **#2** (dónde vive `Workspace`), **#3** (una sola definición de `Check`/`Severity`/`CheckCode` —
> cambia el *catálogo de códigos*, no la regla), **#4** (orden de `Severity` / conteo de `hard_fail`),
> **#6** (sin capa DTO), **#8** (un watcher = único escritor), **#9** (`RelPath` newtype validado),
> **#10** (el store es dueño del DDL). Quedan **superadas por §20** las que dependen del modelo OKF:
> **#5** (forma de `Analysis`: `in_index`/`orphans` desaparecen), **#11** (semántica de query por
> subcadena → lenguaje de expresiones tipado) y las filas de generadores puros / `merge_frontmatter`
> en la medida en que describen `gen_index`/`gen_tag_indexes` (retirados). Las filas **#15–#21** (git)
> quedan **retiradas**, no dormidas: §20 borra el crate `lodestar-vcs`.
>
> **Nota (giro headless §19, 2026-07-22).** Las filas **#15–#21** (git de primera clase) siguen
> siendo **ciertas sobre el crate `lodestar-vcs`**, pero su **exposición en la superficie de producto
> queda revertida** por §19: el crate se conserva dormido y ninguna fachada lo consume. Las filas
> **#1–#14 siguen vigentes tal cual** y son el cimiento del giro (core puro, una verdad computada, un
> contrato de tipos, único escritor, `RelPath` chokepoint, generadores puros, `merge_frontmatter` en
> el core, feature `schemars`). §19 **no relitiga** ninguna decisión #1–#14; las usa.

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
| 15 | ¿Dónde vive git? | Crate `lodestar-vcs`, hermana de `store`; core sin git. **Transporte híbrido**: libgit2 para lo *local* (no ejecuta hooks/config al abrir/indexar = RCE-safe) + binario `git` confinado a la *red* (push/pull/fetch, iniciados por el usuario, heredan su auth). El shell-out nunca corre en open/index. |
| 16 | "Restore soft" podía **perder trabajo sin commitear** | Restore/cambio de rama/merge son no-destructivos *de historial* pero reescriben el working tree → **checkpoint automático** si hay cambios sin commitear; excluyen `log.md` curado; **regeneran** `index`/`tags` tras aplicar. |
| 17 | Marcadores de conflicto pasaban la conformidad | Nuevo check **`OKF-CONFLICT`** (hard-fail) por `<<<<<<<`/`=======`/`>>>>>>>`/`\|\|\|\|\|\|\|`; el gate y la conformidad-por-commit los detectan en las 3 fachadas. |
| 18 | Merge/rebase en curso no detectado | `RepoState` desde `repository.state()`; pill/overlay muestran "resolviendo conflicto" y `commit` se niega sobre índice no-merge. |
| 19 | Pill obsoleto tras commit (no cambia bytes) | Defensa en profundidad: ref-watch del gitdir real (incl. `logs/HEAD`) + update **optimista** con el `Sha` + reconcile al enfocar. El ref-watch es pista, no garantía. |
| 20 | Tipos commit/diff/cache triplicados | Una familia en `core::types` (§4.4); cache de conformidad por **tree-oid** (dedup de reverts) con gate `ruleset_version`; golden cross-fachada. |
| 21 | Contador "sin commitear" usaba `diffSnap` (caro/edición) | **Hash por path** contra el HEAD-map en RAM (O(cambiados)); `OkfDiff` completo perezoso solo al abrir overlay/modo Cambios; LCS con guarda + DP dos-filas/Hirschberg; saltar blobs binarios. |

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
| **Seguridad** | **DOMPurify** en el markdown (no regex casera); escapado de expresiones FTS5; **el shell-out al binario `git`** (solo red) usa argumentos fijos validados, jamás interpola input no confiable y nunca corre en open/index; threat model de una página (webview, MCP confianza-local, zip-slip, path-traversal, subproceso git). |
| **Errores** | Taxonomía fatal/recuperable/transitorio + afford de recuperación; código estable cruzando `CoreError`→`AppError`/exit-code; **supervisar el watcher** (panic → restart + banner, nunca UI obsoleta en silencio). |
| **Config** | Dos niveles: app-global (tema/layout/recents) y **por-bundle** (`lodestar.toml` commiteado: strictness, write policy, locale de artefactos) para que GUI/CLI/MCP coincidan. |
| **Un bundle por proceso** | Asunción documentada + lockfile que elige un único indexador cuando GUI y MCP abren el mismo bundle. Multi-ventana/multi-bundle = no-goal v1. |
| **First-run** | `lodestar init` / "crear bundle" en GUI: scaffold de `index.md` raíz con `okf_version`, `.gitignore` (incluye `.lodestar/`), **`git init` + commit inicial**. En cada `open` de un repo existente se verifica que `.lodestar/` está ignorado (idempotente; oferta "dejar de trackear" si ya estaba trackeada). |
| **Sincronización / remoto** | **push/pull/fetch soportados in-app** vía el binario `git` sobre el **upstream ya configurado** (hereda el auth del usuario: SSH-agent/credential-helpers/tokens). libgit2 **nunca** habla con la red. **`clone` y la gestión de remotos siguen no-goal v1** (el usuario clona/añade remotos con su `git` CLI; lodestar usa el remoto existente). El ref-watch absorbe los cambios de `pull`. |
| **Paridad con `git` CLI** | **Commits van por libgit2**: no corren hooks de commit, no firman (`commit.gpgsign` ignorado), no aplican filtros LFS/`.gitattributes`. v1: commits **sin firmar**; si el repo exige firma, **avisar** en el diálogo y ofrecer commitear vía CLI. **push/pull van por el binario `git`** → sí corren los hooks pre-push/post-merge del usuario y respetan LFS/credenciales (es su repo, acción explícita). El shell-out se **confina a la red**; commit/log/diff nunca lo usan. |
| **Identidad / atribución** | Autor+committer separados; override (`lodestar.toml [identity]`)→git config→fallback marcado. Commits del **agente** (MCP) llevan trailer `Co-Authored-By` distinguible para que `git log`/blame no mientan. `[identity]` se añade al schema de `lodestar.toml`. |
| **CRDT (futuro)** | Documentar que la canonicalización de `build_raw` + LWW por fichero sesga contra un CRDT por-bloque. Mantener el core sin I/O para que un futuro server `axum` reuse la superficie de análisis. |

---

## 13. Versionado (git) — integración de primera clase

> **⚠ Superada por §19 en cuanto a superficie de producto (ratificado 2026-07-22).** El giro a
> *motor headless de integridad semántica* (§19) **retira git de la superficie**: ninguna fachada
> (MCP/CLI) expone commit/rama/push/pull/merge/hooks ni la conformidad-por-commit. **El crate
> `lodestar-vcs` y su mecánica interna (§13.2–§13.6) se conservan como DORMIDOS** — compilan, sus
> tests siguen verdes, `Workspace` puede seguir teniendo los métodos `vcs_*` internamente, pero
> **ningún consumidor los llama**. Esta sección queda como diseño de referencia por si git vuelve a
> la superficie; no describe implementación viva de producto en v2. Los tipos git de `core::types`
> (`Sha`/`CommitRow`/`Branch`/`OkfDiff`…) permanecen en el contrato aunque dejen de exponerse.

El prototipo añadió **"Versiones / historial"**. La implementación real lo eleva a **git de primera clase con
vocabulario directo** (commits, ramas, push/pull): el público objetivo es técnico (desarrolladores), así que
**no** se esconde git tras eufemismos para "quitar complejidad". **Transporte híbrido**: libgit2 para lo local,
binario `git` confinado a la red. Se integra sin romper ninguna decisión ratificada (core puro, único escritor,
snapshot-push, un contrato de tipos).

### 13.1 Terminología

**Vocabulario git directo** (público técnico): la UI dice "commit", "rama", "push", "pull", "merge" — no
eufemismos. El prototipo usaba términos velados ("versión", "línea principal"); el port los reemplaza por los
términos git. Solo quedan como término *propio* los conceptos que **no** son git sino OKF.

| UI de lodestar | git / concepto |
|---|---|
| "commit" / "Hacer commit" (Ctrl/Cmd+S) | commit (no hay paso de "guardar" aparte: el `.md` se escribe atómico al editar) |
| "N sin commitear" / "Limpio" | working tree dirty vs HEAD (menos generados) |
| "rama" (actual · cambiar · crear · merge) | branch (resuelta de HEAD; create/switch/merge locales) |
| "push" / "pull" | push/pull al upstream configurado (vía binario `git`) |
| "restaurar a un commit" | materializar el árbol de un commit al working tree (soft) |
| **"último commit conforme"** | último commit cuyo árbol pasa la puerta OKF (concepto OKF, no git) |
| **"propuesta en revisión"** | concept con `status: review` — **NO** una rama (decisión del modelo OKF, §13.8) |

### 13.2 `lodestar-vcs` (libgit2 local + binario `git` para red)

Dueño único de git, hermana de `store`. Encapsula git2: expone `Sha`/`Branch`, nunca `git2::Oid`. **Transporte
híbrido**: libgit2 para todo lo local (commit/log/diff/branch/merge/restore) — abrir/indexar un bundle ajeno no
ejecuta sus hooks/aliases/`include.path`/fsmonitor (RCE-safe); el binario `git` se invoca **solo** para
push/pull/fetch (red), por un camino aparte (subproceso con args fijos validados, nunca interpola input no
confiable), para heredar el auth del usuario. `git2::Repository` es `!Sync` → vive tras el único escritor
(`Mutex<Vcs>`).

```rust
impl Vcs {
  // --- local (libgit2) ---
  pub fn discover(root: &Path) -> Result<Option<Vcs>>;     // None = sin .git (modo "activar git")
  pub fn init(root: &Path) -> Result<Vcs>;                 // git init + .gitignore + commit inicial
  pub fn status(&self) -> RepoStatus;                      // dirty set + RepoState (merge en curso)
  pub fn log(&self, limit: usize) -> Vec<CommitRow>;       // metadatos baratos (revwalk), sin leer árboles
  pub fn log_for_path(&self, p: &RelPath, limit: usize) -> Vec<CommitRow>;  // con techo de commits escaneados
  pub fn tree_files(&self, sha: &Sha) -> Result<FileMap>;  // árbol de un commit → file-map SIN tocar el working tree
  pub fn commit(&self, msg: &str, author: &Author) -> Result<Sha>;          // stage + commit del working tree
  pub fn branches(&self) -> Vec<Branch>;                   // locales + ahead/behind vs upstream
  pub fn current_branch(&self) -> Option<String>;          // la rama actual; HEAD desacoplado = None
  pub fn create_branch(&self, name: &str, from: Option<&Sha>) -> Result<()>;  // no toca el working tree
  // switch_branch / merge / restore NO los APLICA vcs: devuelven el árbol/file-map destino; la workspace
  //   computa un core::Mutation (diff vs working tree) y lo aplica por el ÚNICO escritor.
  pub fn switch_branch_target(&self, name: &str) -> Result<FileMap>;         // árbol de la rama destino
  pub fn merge_target(&self, name: &str) -> Result<FileMap>;                 // fija MERGE_HEAD en .git (commit de 2 padres) + file-map merged; conflicto → marcadores inline (OKF-CONFLICT + RepoState=Merging)
  // --- red (binario `git`, subproceso confinado) ---
  pub fn pull(&self) -> Result<SyncOutcome>;               // git pull --ff-only; si la rama divergió, aborta limpio → la UI sugiere merge (nunca conflicta in-app)
  pub fn push(&self) -> Result<SyncOutcome>;               // al upstream configurado; rechazo (non-ff) → ok:false
}
```

- **vcs no escribe el working tree en las operaciones locales.** `restore`, `switch_branch` y `merge` devuelven un
  árbol/file-map que la **workspace** convierte en `core::Mutation` y aplica por el único escritor (igual que los
  generadores). `merge` además fija `MERGE_HEAD` en `.git` (vcs es dueño de `.git`, no del working tree). La
  **excepción es `pull`** (subproceso `git`), que muta bytes como **escritor externo**: el watcher (gate blake3)
  reconcilia y el ref-watch absorbe las refs — el único escritor de la cache SQLite se preserva, igual que un `git
  pull` lanzado en la terminal (§9). Blobs binarios/no-UTF8 se **saltan y diagnostican** en `tree_files` (no abortan
  el árbol ni la cache de conformidad).
- **Degradación sin `.git`**: `discover` con techo en el root del bundle (no engancha un repo ancestro como
  `~/.git`); tres estados distintos: sin-repo ("activar git"→`init`), repo-vacío, con-historial.
- **Red confinada al binario `git`**: `pull`/`push` lanzan un subproceso con argumentos fijos (`git pull
  --ff-only` / `git push` sobre el upstream), heredan el entorno de auth del usuario, y **nunca** se ejecutan en
  `open`/`index` (solo por acción explícita). Sin upstream configurado → la UI deshabilita push/pull y remite al
  `git` CLI (clone/añadir remoto siguen fuera de scope, §13.8). **Sin binario `git` en el PATH (o versión
  incompatible)** → push/pull deshabilitados con aviso accionable; las operaciones **locales** (libgit2) siguen
  funcionando.

### 13.3 Diff semántico OKF (puro, en el core)

El módulo `core::diff` (port de `diffSnap/fmDiff/lineDiff/collapseDiff`) es la **única verdad computada** del diff;
lo renderizan igual las fachadas y el frontend (`OkfDiff`, §4.4). vcs da dos file-maps (árbol vs árbol, o HEAD vs
working); el core da el *significado*: frontmatter por-campo (orden `status` primero), cuerpo por LCS con plegado de
contexto, transiciones de ciclo de vida, impacto en el grafo de enlaces, y **segregación de generados** (index/log/
tags no cuentan como edición manual).

- **Rendimiento**: el contador "sin commitear" es comparación de **hash por path** contra el HEAD-map en RAM
  (O(cambiados)), **nunca** `diffSnap`. El `OkfDiff` completo (con LCS) se computa **perezoso**, solo para el
  fichero abierto. El LCS lleva guarda de tamaño (fallback grueso por umbral) y DP en dos filas + Hirschberg
  (mata el muro de memoria O(n·m): un fichero de 10k líneas reservaba ~400 MB).

### 13.4 Conformidad por commit (la pieza estrella)

Cada commit guarda su conformidad — el `confOf(snap)` del prototipo hecho real:
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

1. **Restore / cambio de rama / merge no pierden trabajo.** No-destructivos *de historial* pero **reescriben el
   working tree**: si hay cambios sin commitear, primero hacen un **commit de checkpoint** automático (trabajo
   perdido → "un commit más al que volver"); excluyen el `log.md` curado; **regeneran** `index`/`tags` tras aplicar.
2. **`OKF-CONFLICT`** (hard-fail): marcadores `<<<<<<<`/`=======`/`>>>>>>>`/`|||||||` en cuerpo o frontmatter,
   vengan de un `merge` in-app (libgit2) o de un `git merge`/`pull` conflictivo del CLI externo — antes pasaban la
   conformidad en silencio.
3. **`RepoState`** desde `repository.state()`: detecta un merge/rebase en curso en `.git` — el `merge` in-app
   (libgit2 fija `MERGE_HEAD` → commit de 2 padres) o un merge/rebase del `git` CLI externo. Bloquea el commit y
   avisa "resolviendo conflicto" (en vez de `add_all`+commit sobre índice no-merge → basura). Los **marcadores** los
   caza `OKF-CONFLICT` (gate); `RepoState` cubre el **estado** del repo. (`pull` es `--ff-only` → nunca deja un
   merge a medias.)
4. **Pill nunca obsoleto.** Un commit no cambia bytes → defensa en profundidad: ref-watch del gitdir real (incl.
   `logs/HEAD`, maneja `.git`-como-fichero), update **optimista** con el `Sha`, y reconcile al enfocar.

### 13.7 Fachadas y frontend

> **IPC front↔back retirado de `main`** (giro headless, §19.1): los comandos Tauri, el evento
> `bundle:changed`, el frontend y su contrato `contracts/ipc.yml` se movieron a la rama
> `experimental/ui-desktop`. Lo que sigue es **diseño ratificado de referencia**; la única frontera
> viva en el motor headless es la MCP (§13 git, además, está dormido). No describe superficie activa
> de este repo.

- **Tauri**: `vcs_status` · `vcs_log` · `vcs_log_for_path` · `vcs_diff(a,b,filter?)` · `vcs_diff_working` ·
  `vcs_commit(msg, alsoLog)` · `vcs_restore(sha)` · `vcs_branches` · `vcs_create_branch(name)` ·
  `vcs_switch_branch(name)` · `vcs_merge(name)` · `vcs_pull` · `vcs_push` · `vcs_last_conforming` · `vcs_init`.
  `bundle:changed` crece un campo `vcs` barato (head/branch/ahead/behind/pendingCount/clean); el `OkfDiff`/log
  pesados se piden al abrir.
- **MCP**: `history(concept?)` · `diff(revA,revB)` · `last_conforming_commit` · `when_changed(concept)` ·
  **`commit(message)`** (única escritura del agente: hace checkpoint y recibe la conformidad post-commit → aprende
  "no conforme" y se autocorrige). **push/pull y operaciones de rama quedan fuera del MCP** (sync y topología de
  ramas son acciones humanas deliberadas). Commits del agente con trailer `Co-Authored-By`.
- **CLI**: `log` · `diff` · `last-conforming` · `branch` (list/create/switch) · `merge` · `pull` · `push` ·
  `hooks install` (+ `check --staged/--rev/--range`).
- **Frontend**: el **pill** de git (HEAD/rama/ahead-behind/pendientes) + popover (pendientes, recientes, cambiar de
  rama, push/pull, "restaurar al último conforme"); el **overlay**
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
| Sincronización / remoto | **push/pull/fetch in-app** vía binario `git` sobre el upstream configurado (hereda auth). **clone y gestión de remotos = no-goal** (`git` CLI del usuario). libgit2 nunca toca la red. |
| Firma de commits | **Sin firmar** (commit por libgit2, ignora `commit.gpgsign`); si el repo la exige, avisar y ofrecer commit vía CLI. (`push` sí respeta hooks/firma del lado servidor.) |
| LFS / `.gitattributes` | **commit** (libgit2) no aplica filtros → detectar y **avisar** (no commitear un blob LFS crudo); **pull/push** (binario `git`) sí respetan LFS. Binarios fuera de scope, se saltan. |
| Ramas | **Crear/cambiar/merge locales** desde la app (libgit2; switch/merge reescriben el working tree por el único escritor + checkpoint). Rebase = diferido. |
| Propuestas | `status: review`, **no** ramas/PR — decisión del modelo OKF (aunque ahora haya ramas). Aceptar/rechazar = editar frontmatter. |
| Tags · submódulos · worktrees · repos bare | Diferidos / no soportados v1 (degradan, no crashean). |

---

## 14. Plan de construcción por fases

Cada fase se valida con el arnés de paridad antes de la siguiente.

1. **`lodestar-core` puro** + el contrato de tipos (§4) + **el diff semántico OKF** + arnés diferencial contra el
   prototipo JS. Sale aquí toda la lógica OKF; testeable sin GUI/DB.
2. **`lodestar-cli`** mínimo (`check`/`index`/`tags`/`export`) sobre el core efímero. Ya es útil como gate de CI.
3. **`lodestar-store`** (SQLite/FTS5 + watcher) + test de paridad SQL==core + property test incremental==rebuild.
4. **`lodestar-vcs`** (libgit2: status/log/diff/commit/restore/branch/merge/init + ref-watch · binario `git` para
   push/pull/fetch) + cache de conformidad por commit (tree-oid) + `lodestar check --staged/--rev` y `hooks install`.
5. **`lodestar-workspace`** (handle unificado + bus de eventos + único escritor; compone core+store+vcs; restore con checkpoint).
6. **`src-tauri`** + **frontend Svelte** portando el prototipo verbatim (incl. pill/overlay/modo "Cambios"); `.d.ts` generado; editor multi-escritor.
7. **`lodestar-mcp`** (casi gratis: 4ª fachada sobre la misma workspace, con `commit` para agentes) + golden cross-fachada.
8. Transversales de producto: migración (con replay de historial a git), packaging/updater, i18n, seguridad, config por-bundle, first-run.

---

## 19. Motor headless de integridad semántica (supersede §13 en superficie)

> **Superada por §20 en cuanto al MODELO DOCUMENTAL Y LA SUPERFICIE DE ESQUEMAS.** El giro headless
> de esta sección (motor sin GUI ni git, consumido por agentes vía MCP/CLI; `lodestar-app`; modelo
> transaccional; perfiles) **sigue íntegro y vigente** — §20 lo hereda entero. Lo que §20 supersede
> es: el modelo OKF de §19.3 (`ConceptRef`/`ConceptRevision`/`core::schema`/`DocType`/relaciones
> tipadas), la tool `schema_inspect` de §19.6 (→ `metadata_inspect`) y `.lodestar/schema.yaml` de
> §19.4. El crate `lodestar-vcs`, conservado dormido por §19.1, se **retira** en §20.
>
> **Ratificado 2026-07-22** (puerta 1 de `/planificar`; fuente: `docs/REFACTOR.md`; propuesta:
> `docs/REFACTOR_DISENO_PROPUESTA.md`). Lodestar deja de posicionarse como "editor local-first con git
> de primera clase" y pasa a ser un **motor headless de integridad semántica para bases de conocimiento
> Markdown gestionadas por humanos y agentes**. Los **invariantes #1–#6 de `CLAUDE.md` siguen íntegros**
> y son el cimiento del giro; **no se relitiga** ninguna decisión ratificada #1–#14 de §10.

### 19.1 Posicionamiento

Lodestar **no** compite con Obsidian, **no** es un editor generalista y **no** gestiona git. Ofrece una
capa fiable para: buscar/consultar conocimiento, entender esquemas/tipos/relaciones, detectar
inconsistencias, analizar impacto de cambios, planificar modificaciones semánticas, validar antes de
escribir, publicar cambios recuperables y proteger el workspace frente a estados incoherentes. Flujo:
`descubrir → buscar → leer → analizar → planificar → validar → aplicar → verificar`. Se usa desde Claude
Code, Codex, otros clientes MCP y la CLI, **sin editor, sin GUI y sin git** (`REFACTOR §1, §18`).

**Git sale de la superficie de producto** (decisión ratificada): fuera las tools MCP `history`/
`last_conforming_commit`/`commit` y los subcomandos CLI de `crates/lodestar-cli/src/git.rs`
(`log`/`last-conforming`/`branch`/`switch`/`merge`/`pull`/`push`/`hooks`). El crate **`lodestar-vcs`
se conserva DORMIDO** (§13, cabecera). La UI (`frontend/`, `src-tauri/`) se **retiró de `main`** a la
rama `experimental/ui-desktop` (con su IPC Tauri y el contrato `contracts/ipc.yml`): ya no forma
parte del motor headless; el flujo de desarrollo (`.claude/`, `CLAUDE.md`, `docs/WORKFLOWS.md`) se
actualizó en consecuencia. Su diseño se conserva como referencia en §7.1/§8/§13.7 y en esa rama.

### 19.2 Grafo de crates (con `lodestar-app`)

Decisión **D1 (Opción C, híbrido)**: la **mecánica transaccional** (staging, journal, locks, aplicación
atómica por lotes, crash-recovery) vive en `lodestar-workspace`, **junto al único escritor** (preserva el
invariante #5). Se introduce **`lodestar-app`**, crate **fino** de servicios de caso de uso que ambas
fachadas comparten (REFACTOR §3: "MCP y CLI invocan los mismos servicios de aplicación; no contienen
lógica de dominio"). No arrastra `rusqlite`/`git2`/`tokio`.

```
lodestar-core (PURO)  ◄─ lodestar-store ─┐        lodestar-core ◄─ lodestar-vcs (DORMIDO: sin consumidores)
   ▲  (+ core::schema, WorkspaceRevision) ▼
   └──────────────── lodestar-workspace (ÚNICO escritor + staging/journal/locks/recovery + cache + bus)
                              ▲
                       lodestar-app   (servicios de caso de uso · envelope · mapa de códigos de error)
                          ▲       ▲
                   lodestar-mcp · lodestar-cli     (las dos fachadas del motor headless: 5–15 líneas, CERO dominio)
                   (src-tauri RETIRADO de `main` → rama experimental/ui-desktop)
```

- **`core::schema`** (nuevo módulo, **PURO**): tipo `Schema` (catálogo de `DocType`, campos,
  `requiredFields`, `allowedStatuses`, typed relations, lifecycle, plantillas) + funciones de validación
  que, dado un `Schema` + un `Bundle`, producen `Vec<Check>` (extiende `conform`). La **aplicación de
  plantillas** es generación pura (como `gen_index`/`gen_tag_indexes`). **Leer** `.lodestar/schema.yaml` /
  `.lodestar/templates/` es I/O de `workspace` (patrón `Config::load`); el core nunca abre ficheros.
- **`lodestar-app`**: ensambla `ChangeSet`, conduce plan→validar→aplicar→verificar, construye el
  **envelope** (decisión D3: el envelope es framing de protocolo, no dominio → vive aquí, no en `core`) y
  mapea `CoreError`/`WorkspaceError` → **códigos de error** estables.

### 19.3 Tipos nuevos (invariante #4: UNA vez en `core::types`)

Se congelan en `lodestar-core::types` (salvo el envelope, que va en `lodestar-app`):

- `ConceptRevision` = `blake3:<hex>` del contenido en disco de un `.md`. **Eleva** el `WriteOutcome.hash`
  (ya `blake3::hash(raw)`) y el gate de la cache (§5) de gate interno a **identidad expuesta**.
- `WorkspaceRevision` = hash raíz determinista sobre `writableRoots`: ordenar paths (`RelPath: Ord`) →
  hash de cada contenido → combinar `path+hash` → hash raíz. **Independiente** de mtime, orden de fs,
  cachés/índices, **todo `.lodestar/`** (canónico y runtime), `referenceRoots` e ignorados. Lo computa el
  **core** (función pura; invariante #3).
- `ConceptRef { path: RelPath, id: Option<ConceptId> }` — **path** como identidad primaria; `id` opcional
  y diferido (IDs obligatorios = no-goal, REFACTOR §16).
- `ChangeSet { id, base_revision: WorkspaceRevision, operations: Vec<NormalizedOperation>, plan_hash,
  risk: RiskAssessment, semantic_diff: SemanticDiff, validation: ValidationReport, expires_at }`.
- `NormalizedOperation` — enum de las 11 ops resueltas a escrituras (`create`/`patch_frontmatter`/
  `replace_body`/`edit_section`/`replace_text`/`move`/`delete`/`add_relation`/`remove_relation`/
  `transition_status`/`apply_fix`); reutiliza `FrontmatterPatch` (merge-patch RFC 7386, null-borra).
- `RiskAssessment { level, reasons }` — lógica pura nueva alimentada por backlinks/blast-radius.
- `SemanticDiff` — **reutiliza `OkfDiff`** (`core::diff`, port de `diffSnap`), ampliado con
  `diagnosticsIntroduced`/`diagnosticsResolved`.
- `ValidationReport { conformant, summary{errors,warnings,info}, diagnostics: Vec<Check> }` — sobre
  `Analysis.hard_fail`/`warn_count`.
- `ChangeReceipt { id, change_set_id, previous_revision, result_revision, changed_paths, semantic_diff }`.
- **Códigos de error** (§13 de REFACTOR): enum estable en `core::types` (patrón `CheckCode`, wire por
  `#[serde(rename)]`): `WORKSPACE_NOT_FOUND`, `WORKSPACE_RECOVERY_REQUIRED`, `CONCEPT_NOT_FOUND`,
  `AMBIGUOUS_REFERENCE`, `REVISION_CONFLICT`, `PLAN_STALE`, `PLAN_EXPIRED`, `PERMISSION_DENIED`,
  `INVALID_SCHEMA`, `NONCONFORMANT_RESULT`, `INBOUND_LINKS_EXIST`, `RELATION_CONSTRAINT_VIOLATION`,
  `WRITE_CONFLICT`, `RESULT_TOO_LARGE`, `RECOVERY_FAILED`, `INTERNAL_IO_ERROR`.

**Extensión del tipo `Check`** (aditiva, sin forkear — invariante #4): gana campos **opcionales**
`id: Option<_>`, `range: Option<Range>` (`startLine`/`endLine`), `related: Vec<_>`,
`fixes: Vec<Fix{fixId,title,safe}>`. Los 15 checks OKF actuales los dejan vacíos/None.

**Nuevas familias de `CheckCode`** (decisión D-CheckCode): variantes **estáticas acotadas** para los
diagnósticos schema-driven — `SCHEMA-REQFIELD`, `SCHEMA-STATUS`, `REL-TARGET`, `REL-CARD`, `REL-TYPE` —
aditivas y con clave i18n por código (§12). **No** hay espacio de códigos dinámico. El *qué* concreto
(qué campo, qué relación) va en `targets`/`msg`/`related`.

### 19.4 Config nueva y separación canónico vs runtime

Decisión **D4/D5**. La config por-bundle migra de `lodestar.toml` a **`.lodestar/config.yaml`** (YAML
unificado, idiomático con el frontmatter del bundle):

```yaml
workspace:
  writableRoots:   [knowledge]        # Lodestar puede modificar (transacciones)
  referenceRoots:  [src, tests]       # visibles para validación, NUNCA modificables por el MCP
  ignored:         [node_modules, target, dist, .git, .lodestar/runtime]
gate:
  blockWarnings:   false              # strictness (antes [gate] block_warnings)
transactions:
  retainReceiptsFor: 24h
  maximumReceipts:   20
# identity: DORMIDA (git fuera de superficie; se conserva por si vcs vuelve)
```

> **Actualización E15-H08**: `lodestar.toml` **ya no existe** (borrado; cierra `DECISIONES.md §8`),
> así que `.lodestar/config.yaml` es el único fichero de configuración del motor. El esquema de
> arriba se **amplía** con dos secciones que documentan `§20.5` y `§20.9`: `discovery`
> (`include`/`exclude`/`respectGitignore`/`respectLodestarIgnore`/`followSymlinks`/
> `maxDocumentBytes`) y `validation` (severidad por familia de diagnóstico), más
> `transactions.rejectNewErrors`/`allowExistingErrors`. `validation` y la política de cambios **solo
> se cargan**: aplicarlas es E20. `workspace.root` **no** se implementa (circular, `§20.5`).

`.lodestar/` se parte en **dos naturalezas**:

- **Canónico / versionado** (entra a git, pero **fuera** de `WorkspaceRevision` y del índice de
  conceptos — es *config del workspace*, no *conocimiento*): `.lodestar/config.yaml`,
  `.lodestar/schema.yaml`, `.lodestar/templates/`.
- **Runtime / desechable** (gitignored, como hoy `index.db`): `.lodestar/runtime/` (plans/, receipts/,
  journal, `audit.jsonl`) + `.lodestar/index.db`.

Consecuencia: el `.gitignore` deja de ignorar `.lodestar/` entero y pasa a ignorar **solo**
`.lodestar/index.db` + `.lodestar/runtime/`. Invariante #1 intacto: los `.md` de `writableRoots` son la
única fuente de verdad del conocimiento; la config es config.

### 19.5 Modelo transaccional (§5 de REFACTOR) — mecánica en `workspace`

Una "transacción" no es de BD: los `.md` siguen siendo la verdad. Es **semántica transaccional
recuperable**: staging completo → validación previa → lock de workspace → control optimista de
concurrencia (`expectedRevision`/`expectedWorkspaceRevision`) → write-ahead journal → reemplazo atómico
por fichero (el `write_atomic` de §6, en bucle) → copias de recuperación → recuperación tras cierre/fallo
→ validación posterior. **Todo pasa por el único escritor** (invariante #5); staging vive en
`.lodestar/runtime/staging/` (no es el árbol canónico). Al abrir, un journal incompleto dispara la
estrategia determinista (completar o restaurar) **antes** de servir lecturas;
`workspace_status.recovery.pendingTransaction` lo expone y `WORKSPACE_RECOVERY_REQUIRED` bloquea
escrituras hasta resolver. Lodestar **no** asume acceso exclusivo: recalcula/invalida revisiones ante
escrituras externas (REFACTOR §5.3).

### 19.6 Superficie MCP 13 → 10 y perfiles

Diez tools (`REFACTOR §8`): **READ** `workspace_status` · `knowledge_search` · `knowledge_get` ·
`schema_inspect` · `graph_query` · `impact_analyze`; **VERIFY** `knowledge_check`; **CHANGE**
`change_plan` · `change_apply` · `change_revert`. Migración desde las 13 actuales:

| Tool actual | Destino |
|---|---|
| `find_backlinks`/`find_orphans`/`find_dangling`/`neighborhood` | `graph_query(operation=…)` (reusa `Bundle::neighborhood`, `Store::blast_radius`) |
| `conformance_check` | `knowledge_check` (scopes workspace/concept/paths/affected) |
| `query` | `knowledge_search` (filtros, snippets, paginación por cursor) |
| `create_concept`/`update_frontmatter` | `change_plan` + `change_apply` (ops `create`/`patch_frontmatter`) |
| `generate_index`/`generate_tag_indexes` | **CLI** (`lodestar index`/`tags`) + **auto-regen dentro de `change_apply`** cuando el cambio afecta a index/tags (decisión D6a) |
| `history`/`last_conforming_commit`/`commit` | **ELIMINADAS** (git fuera de superficie) |

`impact_analyze` reutiliza el **blast-radius** del store (`synth::blast_radius`) y `neighborhood`.
**Perfiles** (§12 de REFACTOR): `readonly` = las 7 de lectura/verificación; `standard` = añade las 3 de
cambio. Se eligen **al arrancar** (`lodestar-mcp --profile readonly|standard`). **Política** de
conformidad al arrancar (`--policy strict`, `strict` por defecto): no hay `allow_nonconformant` por
llamada (seguridad §19.7). **Transporte** (decisión D6b): se mantiene **stdio** (DECISIONES §3) y se
activa **`outputSchema`** derivado con la feature `schemars` (ya preparada, §10 fila 14); `rmcp`
**diferido**. `contracts/mcp.yml` se **reescribe** 13→10 y el guardián de contrato lo vigila.

**Envelope común** (en `lodestar-app`): `{ ok, workspaceRevision, summary, data, diagnostics, warnings,
resourceLinks }`; `summary` es texto compacto para el modelo.

### 19.7 Seguridad (§14 de REFACTOR) — simplificada

`RelPath` (§4.1) sigue siendo el **chokepoint sintáctico** (rechaza absolutas/`..`/backslash/unidad
Windows). Se **añade** (aditivo) una comprobación **semántica** de nivel workspace: (1) el path resuelto
cae bajo un `writableRoot` para escribir (bajo un root visible para leer); (2) **guarda de symlinks** por
canonicalización + verificación de contención. El servidor arranca con un único root, no permite cambiar
de workspace por tool, **no ejecuta comandos, no accede a red, no conoce git** (el crate `vcs` queda sin
consumidores → la superficie no lanza procesos ni toca la red; el threat model de §12 se **simplifica**).
Auditoría local en `.lodestar/runtime/audit.jsonl` (runtime, no conocimiento).

### 19.8 Plan de fases → épicas

Épicas nuevas **09–14** (las 00–08 quedan como están; `requirements/`):

| Épica | Fase REFACTOR | Foco |
|---|---|---|
| **E9** — Reducción de alcance | 0 (§16) | Retirar git de superficie; congelar UI en `.claude/`/docs; `.lodestar/config.yaml` + separación canónico/runtime; escribir §19; reposicionar README/CLAUDE |
| **E10** — Esquemas + lectura headless | 1 | `core::schema` puro; `ConceptRevision`/`WorkspaceRevision`/`ConceptRef`; extensión de `Check`; envelope + códigos de error; crate `lodestar-app`; `workspace_status`/`knowledge_search`/`knowledge_get`/`schema_inspect`/`knowledge_check` |
| **E11** — Grafo e impacto | 2 | `graph_query` (consolida grafo); `impact_analyze` (blast-radius); typed relations + validación de `referenceRoots` |
| **E12** — Planificación | 3 | `ChangeSet`/`NormalizedOperation`/`RiskAssessment`/`SemanticDiff`/`ValidationReport`; `change_plan` (sin escribir); 11 ops; optimistic concurrency |
| **E13** — Publicación recuperable | 4 | Staging · journal · locks · copias de recuperación · crash-recovery · `change_apply` · `change_revert` · `ChangeReceipt` · `audit.jsonl` |
| **E14** — Integración software + evaluación | 5+6 | Validación de paths de código en CI; knowledge checks en CI; benchmarks (§17); tokens; concurrencia; recuperación |

Cada fase se valida antes de la siguiente; los criterios de aceptación se alimentan del **benchmark
funcional** (`REFACTOR §17`).

---

## 20. Workspace Markdown universal (supersede §4, §5 y §19.3 en modelo documental)

> **Ratificado 2026-07-23** (puerta de diseño; fuente: `docs/REFACTOR_PHASE_2.md`). Lodestar deja de
> exigir **OKF** como formato documental y pasa a operar sobre **cualquier red de ficheros Markdown
> contenida en un proyecto**. El giro headless de §19 (motor sin GUI ni git, `lodestar-app`, modelo
> transaccional, perfiles) **se hereda íntegro**: esta sección cambia *qué* se modela, no *cómo* se
> expone ni *cómo* se escribe.

### 20.1 Definición del producto

> Un motor local y transaccional para que agentes de IA puedan descubrir, consultar, comprender y
> modificar de forma segura una red arbitraria de documentos Markdown contenida dentro de un proyecto.

La unidad fundamental deja de ser el *bundle OKF* y pasa a ser el **workspace**:

```
Workspace
├── root  (el cwd, o --root)
├── discovery policy · write policy
├── document inventory      (todos los .md descubiertos recursivamente)
├── metadata index          (cualquier propiedad YAML, anidada, sin lista cerrada)
├── link graph              (enlaces Markdown estándar resueltos por PATH)
├── diagnostics · search index · transaction state
```

El valor diferencial **no depende de un formato propio**: descubrimiento global, consultas
estructuradas sobre frontmatter, grafo, backlinks, análisis de impacto, planificación de cambios,
validación previa, escrituras atómicas, auditoría, recovery y rollback.

**Arranque sin ceremonia** (criterio de aceptación central): `cd my-project && lodestar-mcp` funciona.
No es obligatorio `lodestar init`, ni `.lodestar/config.yaml`, ni frontmatter, ni `type`, ni `status`,
ni `index.md`. La configuración solo sirve para **limitar** descubrimiento, escrituras o diagnósticos
— nunca para convertir un workspace en válido.

### 20.2 Invariantes del modelo (los 20 de `REFACTOR_PHASE_2 §Invariantes`)

Se **añaden** a los invariantes #1–#6 de `CLAUDE.md`, que siguen íntegros. Los que fijan diseño:

1. Ningún path público es absoluto; ninguna operación escapa del workspace (sigue siendo `RelPath`,
   §4.1, el chokepoint sintáctico + la guarda semántica de §19.7).
2. Todo documento descubierto tiene una ruta canónica única; **todo enlace se resuelve por path**,
   nunca por título, basename, alias o similitud. Sin resolución heurística ni ambigua.
3. El frontmatter **nunca** es obligatorio y sus claves **no** tienen semántica impuesta.
4. Los tipos YAML se respetan **sin coerción implícita** (`priority >= "high"` es un error de tipo).
5. Los documentos aislados **no** son errores; `index.md` y `README.md` **no** tienen trato especial.
6. La estructura de carpetas **no** altera el significado de los documentos.
7. El store se reconstruye por completo desde los ficheros; análisis puro y store son equivalentes.
8. El proyecto **no** depende de sintaxis de Obsidian (sin wikilinks, embeds, block refs ni aliases).

### 20.3 Terminología retirada de la API pública

`OKF` · `bundle` · `concept` · `conformance` · `okf_version` · `OKF-IDX` · `OKF-LOG` · `in_index` ·
`concept type` · `concept status`.

| Anterior | Nueva |
|---|---|
| Bundle | Workspace |
| Concept / ConceptRef / ConceptSummary / ConceptRevision | Document / DocumentRef / DocumentSummary / DocumentRevision |
| OKF diff | Semantic diff |
| Conformance / Conformant | Validation / Valid |
| Orphan | Isolated document |
| Bundle revision | Workspace revision |

### 20.4 Modelo documental (supersede §4.1 en frontmatter y clases de fichero)

```rust
pub struct Document {
    pub path: RelPath,                        // §4.1 sin cambios (newtype validado)
    pub raw: String,
    pub frontmatter: Option<ParsedFrontmatter>,
    pub body: String,
    pub content_hash: ContentHash,
}

/// El frontmatter es metadata ARBITRARIA del usuario. Sin campos conocidos, sin lista cerrada,
/// sin conversión automática de tipos, sin borrado de claves desconocidas.
pub struct ParsedFrontmatter { pub value: serde_yaml::Value, pub raw: String, pub span: Range<usize> }

/// El agregado analizable, independiente del sistema de ficheros (sustituye a `Bundle`).
pub struct DocumentSet { pub documents: FileMap }
```

**Desaparecen**: `FileKind` (`Index`/`Log`), `KNOWN_FM`, los 7 campos tipados de `Frontmatter`,
`RelPath::is_reserved`/`concept_id`, `okf_version`, `in_index`, `index_refs`, `src_is_index` y la
pertenencia determinada por índices.

**Título derivado** — `frontmatter.title` → primer heading H1 → nombre del fichero. Es **solo una
heurística de presentación**: `title` no se convierte en propiedad reservada.

**Edición de frontmatter** — la operación genérica es `patch_frontmatter` (`set` + `remove`), que
modifica solo las claves pedidas, preserva las demás, no reordena innecesariamente, mantiene el
cuerpo intacto y **distingue explícitamente asignar `null` de eliminar una clave**. El plan debe
declarar si el bloque se reserializará entero.

### 20.5 Descubrimiento (§3 de REFACTOR_PHASE_2)

La raíz es `--root` si se da, si no `std::env::current_dir()`, canonicalizada al arrancar y **fija
durante toda la sesión**. Todas las rutas públicas son relativas a ella.

> **`workspace.root` en la config NO se implementa** (E15-H08). `REFACTOR_PHASE_2 §Fase 2` lo
> sugiere como configuración opcional, pero es **circular**: el fichero vive en
> `<root>/.lodestar/config.yaml`, luego hay que conocer la raíz para leerlo. La raíz sale
> exclusivamente de `--root` o del cwd.

Política por defecto:

```yaml
discovery:
  include: ["**/*.md"]
  exclude: [".git/**", ".lodestar/**"]
  respectGitignore: true
  respectLodestarIgnore: true
  followSymlinks: false
```

> **Corrección (E15-H07)**: `REFACTOR_PHASE_2 §Fase 3` sugiere `.lodestar/runtime/**` en su
> «política recomendada». Se excluye **`.lodestar/` entero** por una **invariante de consistencia**:
> *todo documento del inventario debe contar para la `WorkspaceRevision`*. Si no, sería nodo del
> grafo, analizable y escribible, con cambios que nunca mueven la revisión — el control optimista
> dejaría de protegerlo en silencio. Y la revisión **no puede** dejar de excluir `.lodestar/`
> (decisión **D5**): `StagingDir` materializa ahí un árbol `.md` completo —copias de los documentos
> cuya escritura está guardando—, así que si contara, `reverify_base_revision` fallaría *a causa del
> apply en curso*: el motor transaccional invalidaría su propia base al preparar la escritura. Lo
> mismo con las copias de recuperación. `.lodestar/` es el **plano de control** (config, cache,
> runtime), nunca conocimiento del usuario. Tras E20 —que retira `schema.yaml` y los templates— ahí
> no queda nada más.

Sin profundidad máxima artificial. Restricciones iniciales: documentos UTF-8, paths representables,
tamaño máximo configurable, symlinks desactivados. Se detectan **colisiones de capitalización**.

### 20.6 Enlaces (supersede `resolve_link` de §4)

Solo **Markdown estándar**: inline `[t](p.md)`, con fragmento `[t](p.md#s)`, de referencia
`[t][id]` + `[id]: ../p.md`, anchors del propio documento `[t](#s)` y URIs externas. Algoritmo:
parsear con el parser Markdown → separar path/query/fragment → detectar URI externa y self-anchor →
resolver contra el directorio del documento origen → normalizar `.`/`..` → verificar contención en
el workspace → resolver contra el inventario → clasificar → registrar href original **y** destino
normalizado.

```rust
pub enum LinkTarget {
    Document(RelPath),        // otro .md del inventario → arista del grafo
    WorkspaceFile(RelPath),   // fichero del proyecto que NO es .md (p. ej. código): existe, pero NO es nodo
    ExternalUri(String), SelfAnchor(String), Missing(RelPath), EscapesWorkspace,
}
```

**Prohibido**: buscar por basename o título, añadir `.md` automáticamente, resolver un directorio
como `index.md`, tratar `README.md` como fallback, interpretar aliases o resolver ambigüedades por
heurística. **Sin soporte de Obsidian** (wikilinks, embeds, block refs).

### 20.7 Grafo y análisis (supersede §4.1 `Analysis`)

Nodos = **todos** los documentos Markdown descubiertos. Aristas = enlaces resueltos entre ellos.

```rust
pub struct Analysis {
    pub documents: Vec<RelPath>,
    pub outgoing: BTreeMap<RelPath, Vec<ResolvedLink>>,
    pub incoming: BTreeMap<RelPath, Vec<LinkReference>>,
    pub isolated: Vec<RelPath>,          // sin enlaces internos entrantes NI salientes
    pub dangling: Vec<DanglingLink>,
    pub diagnostics: BTreeMap<RelPath, Vec<Diagnostic>>,
}
```

Un **documento aislado no es inválido**: es una propiedad consultable (`graph.isolated = true`) que
no genera warning por defecto.

### 20.8 Lenguaje de consulta (supersede §4.3)

La DSL de tokens con semántica de subcadena se sustituye por un lenguaje de expresiones **tipado**
sobre cualquier propiedad YAML, con dot-notation para propiedades anidadas.

- **Comparación** `= != > >= < <=` · **texto** `contains starts_with ends_with` · **listas**
  `contains contains_any contains_all` · **lógica** `and or not (…)` · **existencia** `has(x)`
  `missing(x)` (incluido `has(frontmatter)`).
- **Namespaces**: `frontmatter.*` (abreviable — `status = "x"` ≡ `frontmatter.status = "x"`),
  `document.path|title|has_frontmatter`, `graph.backlinks|outgoing_links|dangling_links|isolated`.
  Las propiedades calculadas **exigen** namespace explícito.
- **Sin coerción implícita** entre string/número, string/booleano, escalar/lista, lista/objeto. La
  heterogeneidad de tipos de una propiedad es inspeccionable y comunicable.

> **Aviso de implementación (E16-H01)**: el evaluador de comparaciones debe ir **siempre** sobre
> `ParsedFrontmatter::get` (que devuelve el `serde_yaml::Value` con su tipo), **nunca** sobre
> `get_text`. `get_text` renderiza escalares a `String` para las columnas de cache y los DTO de
> presentación; construir las comparaciones encima haría que todo se comparase como texto y el
> invariante 4 de `§20.2` —`priority >= "high"` es un error de tipo— desaparecería **sin que ningún
> test lo notara**, porque para fechas y números ISO el orden lexicográfico suele coincidir. Es la
> vía por la que puede volver a colarse la coerción implícita que `js_string` tenía y E16-H01
> retiró.
- **Un solo AST** (`Expression`: `Comparison`/`Function`/`And`/`Or`/`Not`): la consulta textual
  (`where`) y el filtro estructurado (`filter`) se traducen al mismo AST y **producen exactamente el
  mismo resultado**.

### 20.9 Validación genérica (supersede §4.1 en códigos)

`knowledge_check` responde *"¿puede Lodestar interpretar y modificar este workspace de forma
consistente y segura?"*, **no** *"¿cumple el workspace una especificación documental?"*.

Deja de ser error: falta de frontmatter, de `type`, de `status`, formato de `tags`, ausencia en un
índice, falta de `okf_version`, documento aislado, estructura de headings, transiciones de estado y
relaciones no tipadas. Catálogo mínimo:

| Código | Significado |
|---|---|
| `FM-UNCLOSED` / `FM-YAML-INVALID` | Frontmatter sin cierre / YAML inválido |
| `DOC-CONFLICT-MARKER` / `DOC-NOT-UTF8` / `DOC-TOO-LARGE` | Marcadores de merge / no UTF-8 / sobre el límite |
| `PATH-NOT-UTF8` / `SYMLINK-UNSUPPORTED` | Ruta no representable / symlink no admitido |
| `LINK-TARGET-MISSING` / `LINK-ESCAPES-WORKSPACE` / `LINK-CASE-MISMATCH` | Destino inexistente / fuera del root / capitalización no portable |

**Política de cambios** (`validation` + `transactions` en la config): `allowExistingErrors: true` —
Lodestar trabaja en un repositorio que ya tiene problemas — junto a `rejectNewErrors: true` — un
cambio no introduce errores nuevos ni empeora los existentes, y una reparación parcial se puede
aplicar.

### 20.10 Superficie MCP (supersede §19.6 en una tool)

Diez tools, con **un solo cambio** respecto de §19.6: `schema_inspect` → **`metadata_inspect`**
(catálogo de propiedades con `presentIn`/`inferredTypes`, inspección de una propiedad con sus valores
y frecuencias, y soporte de propiedades anidadas `service.tier`, `release.target.date`). Permite a un
agente comprender las convenciones de una base desconocida **sin necesitar un schema**.

`knowledge_search` acepta `where` (textual) y `filter` (estructurado) — equivalentes por §20.8 — y
combina full-text, restricción por paths, filtros de metadata y propiedades calculadas de documento y
grafo.

### 20.11 Operaciones transaccionales (supersede §19.5 en el catálogo de ops)

El motor transaccional **no cambia conceptualmente**: `WorkspaceRevision`, `DocumentRevision`, hashes
de contenido, plan inmutable, snapshot de precondiciones, staging, journal, escritura atómica,
recovery, receipt y revert se conservan tal cual — aplicados a Markdown genérico en vez de a
documentos conformes con OKF.

Cambia la **validación previa**: de *"¿el resultado es conforme con OKF?"* a *"¿es parseable? ¿queda
dentro del workspace? ¿respeta la política de escritura? ¿introduce diagnósticos nuevos? ¿coincide con
las revisiones del plan? ¿mantiene consistencia entre inventario, store y grafo?"*.

Ocho operaciones **universales** — `create_document`, `patch_frontmatter`, `replace_body`,
`replace_text`, `edit_section`, `move_document`, `delete_document`, `apply_fix` — y se **eliminan** las
semánticas (`add_relation`, `remove_relation`, `transition_status`, `deprecate`, `replace_concept`):
una relación es un enlace Markdown y un estado es una propiedad arbitraria del frontmatter.

- **Selecciones masivas por consulta**: `{selection: {where: …}, operation: {…}}` →
  `query → documentos → snapshot de revisiones → semantic diff → impact → validation → plan → apply → receipt`.
- **`move_document`** con `rewriteInboundLinks`: encuentra los backlinks, recalcula el enlace relativo
  desde cada origen, reescribe **solo el destino** conservando label y fragmento, y aplica todo como
  una única transacción lógica.
- **`delete_document`** exige **política explícita** (rechazar si hay backlinks · permitir enlaces
  rotos · eliminar referencias · sustituir referencias). Nunca se elige una automáticamente.

La **revisión del workspace** depende, como mínimo, de: rutas Markdown incluidas, hash de cada
documento, configuración de descubrimiento, configuración de escritura, versión del parser y versión
del esquema del store.

### 20.12 Store v2 (supersede §5 en DDL)

El índice SQLite sigue siendo **derivado y desechable**: se incrementa `USER_VERSION` y se reconstruye
por completo — sin migración de datos OKF. Modelo conceptual: `documents(path, title, body, raw,
frontmatter_json, content_hash)` · `metadata(document_path, field_path, value_json, value_type)` ·
`links(source_path, raw_href, target_kind, target_path, fragment, resolved)` ·
`diagnostics(document_path, code, severity, message, range_json)`.

La metadata se indexa **recursivamente** por field path (`service.name`, `service.tier`), conservando
valor JSON original y tipo. FTS indexa path, título derivado, body y valores textuales del
frontmatter — **sin depender** de campos concretos como `type`, `status` o `tags`.

### 20.13 Migración de repositorios OKF existentes

**No se modifican destructivamente los documentos anteriores.** `type: decision` / `status: accepted`
se conservan exactamente y pasan a ser metadata normal, consultable. `index.md` y los índices de tags
sobreviven como documentos Markdown normales (ya no determinan pertenencia, ni versión, ni evitan
aislamiento, ni son catálogo obligatorio). `okf_version` se conserva como metadata desconocida y se
ofrece como **recomendación de limpieza, no como error**. El índice SQLite se elimina y se
reconstruye. Se ofrece un diagnóstico opcional `lodestar migrate-from-okf --dry-run` que **no
modifica ficheros**.

**Se retiran del repo** (decisión del usuario, 2026-07-23): el crate `lodestar-vcs` (dormido desde
§19.1), `core::schema` con `DocType`/relaciones tipadas/`.lodestar/schema.yaml`, `core::generate` con
los subcomandos `init`/`index`/`tags`, `export`/`import` zip, y el prototipo JS como spec de
comportamiento (con su arnés diferencial: la spec pasa a ser `docs/REFACTOR_PHASE_2.md`).

### 20.14 Plan de fases → épicas

Épicas **15–22** (las 00–14 quedan como están; `requirements/`). Corresponden a los 11 PRs de
`REFACTOR_PHASE_2 §Orden de implementación`, con dos ajustes justificados:

| Épica | PRs | Foco |
|---|---|---|
| **E15** — Workspace universal | 0 (retirada) + 1 | Borrado de OKF sin sustituto · `cwd` como root · `--root` · descubrimiento recursivo · seguridad de paths · fixtures arbitrarios |
| **E16** — Modelo documental genérico | 2 | `Document`/`DocumentSet` · frontmatter YAML arbitrario · título derivado · diagnósticos mínimos |
| **E17** — Enlaces y grafo universal | 3 + 4 | Parser de enlaces · `LinkTarget` · escapes · case mismatch · `Analysis` nueva · isolated/dangling |
| **E18** — Store v2 | 5 | DDL nuevo · metadata anidada · links genéricos · cold rebuild · paridad core/store |
| **E19** — Lenguaje de consulta | 6 | Parser · AST · type checking · namespaces · filtro JSON equivalente |
| **E20** — Inspección y validación genéricas | 7 + 8 | `metadata_inspect` (retira `core::schema`) · política `rejectNewErrors`/`allowExistingErrors` |
| **E21** — Contrato MCP y transacciones genéricas | 9 + 10 | Contrato nuevo · 8 operaciones universales · selecciones masivas por consulta |
| **E22** — Migración y limpieza pública | 11 | `migrate-from-okf --dry-run` · docs · README · publicación incompatible |

**Ajuste 1 — E17 fusiona los PRs 3 y 4**: el grafo se construye directamente de los enlaces resueltos;
separarlos obliga a un `Analysis` intermedio que nadie consume.

**Ajuste 2 — la validación genérica se adelanta del PR 8**: al retirar los campos tipados del
frontmatter (PR 2) y la semántica de `index.md` (PR 3/4), los checks `OKF-TYPE`/`OKF-IDX`/`OKF-LOG`/
`ORPHAN` se quedan sin nada sobre lo que compilar. `conform` se reduce al catálogo mínimo de §20.9 ya
en E16/E17; E20 aporta solo la **política** y la semántica nueva de `knowledge_check`.

**Ruptura declarada**: v0.3.0 es **incompatible** con v0.2.x. `v0.2.0` queda como última versión OKF.
