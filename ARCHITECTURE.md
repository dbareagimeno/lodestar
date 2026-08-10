# lodestar — Arquitectura

> Editor local-first de bases de conocimiento en formato **OKF** (Open Knowledge Format):
> un directorio de `.md` con frontmatter YAML. "Solo ficheros": legible por humanos y
> agentes, versionable en git, sin SDK. Este documento define la implementación real a
> partir del prototipo de `prototype/index.html`.

## 1. Stack

> **⚠️ TABLA DE ÉPOCA (v0.2), no el stack vigente.** Es la primera tabla del documento y describe el
> producto **antes** del giro headless (`§19`) y de la migración a Markdown universal (`§20`).
> Siguen siendo ciertas las filas de **Backend/Rust**, **Cache SQLite/FTS5**, **Watcher `notify`**,
> **Fachada agentes (MCP)** y **Fachada CI (CLI)** — salvo que `rmcp` sigue sin adoptarse
> (`decisiones §3`) y que la semántica ya no es «OKF» sino Markdown universal (`§20`).
> **Han dejado de ser ciertas**: «Shell de escritorio (Tauri v2)» y «Frontend (Svelte 5 + Vite)»
> —la UI se retiró de `main` a la rama `experimental/ui-desktop` con el giro headless (`§19.1`)— y
> **«Versionado: git»**, que salió de la superficie en `E9-H02` y del repo en `E15-H01`: no existe
> `lodestar-vcs`, ni `git2`, ni conformidad-por-commit (`§20.13`). Lo único que sobrevive de git es
> `workspace/src/gitignore.rs`, que trata el `.gitignore` como texto plano.

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
   definen **una vez** en `lodestar-core` y cruzan a las fachadas **sin capa DTO paralela**.
   *(La regla sigue vigente; la frase de época decía «cruzan a Tauri/MCP/CLI» y que el `.d.ts` se
   genera con ts-rs/specta. **Ya no hay espejo TS**: desapareció al retirar la UI a
   `experimental/ui-desktop`, y con él la nota de `decisiones §4`. Lo que sí se deriva de los tipos
   Rust es el **JSON Schema** de los `outputSchema` del MCP, vía `schemars`.)*

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
  pub fn merge_frontmatter(&self, p: &RelPath, patch: FrontmatterPatch) -> WriteOutcome; // merge-patch RFC 7386: null ELIMINA la clave
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

> **Las dos REGLAS de abajo siguen vigentes** (único watcher/único escritor —invariante #5— y
> `WorkspaceError` unificado); el **boceto de API** que las sigue es de v0.2 y se lee como
> **historia**, no como firma actual: `lodestar-vcs` y todos los métodos git se **borraron** en
> `E15-H01` (§20.13), `generate_index`/`generate_tag_indexes` en `E15-H02` y `export` en `E15-H03`,
> `BundleSnapshot` es hoy `WorkspaceSnapshot` y `Bundle` es `DocumentSet` (§20.3/§20.4), y
> `open_ephemeral` se **retiró en `E23-H12`**: `open` ya no escribe nada en el proyecto ajeno
> (ni `.gitignore` ni `.lodestar/runtime/`), así que abrir es hermético por defecto y el modo dejó
> de ser un modo. La superficie real vive en `crates/lodestar-workspace/src/lib.rs`.

Compone `lodestar-core` (puro) + `lodestar-store`. Es lo que ven las fachadas. Reglas:

- **Un solo watcher por proceso** que posee el **único escritor** de SQLite. Los comandos
  **nunca** escriben la cache directamente: escriben el `.md` (atómico temp+rename) y dejan que el
  watcher reconcilie. Esto elimina la carrera de doble-escritor.
- **Echo-suppression** = el hash blake3 de la cache es la única autoridad; el `hash` se expone en
  cada DTO de lectura/escritura para que el editor de Svelte distinga su propio echo de una edición externa.
- Error unificado `WorkspaceError` que envuelve `CoreError` + `CacheError` (las fachadas mapean a su exit code / toast).

```rust
impl Workspace {
  pub fn open(root: &Path) -> Result<Self, WorkspaceError>;   // abre/crea cache, arranca watcher, Vcs::discover
  // `open_ephemeral` (sin cache, «CLI hermético») se RETIRÓ en E23-H12: `open` dejó de tener
  // efectos secundarios sobre el proyecto, así que quedó byte a byte igual que él.
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
  `conformance_check(path?)` · `query(dsl)` · `create_concept`(validado) · `update_frontmatter`(validado, patch merge-patch RFC 7386: `null` elimina la clave) ·
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
| 6 | ¿Capa DTO paralela (CheckDto/AnalysisDto)? | **No.** Un solo esquema de wire. *(La regla sigue vigente; lo de época es el `.d.ts` generado desde Rust y la DTO de Tauri: no hay espejo TS desde que la UI se fue a `experimental/ui-desktop`. Lo que se deriva hoy es el JSON Schema de los `outputSchema` vía `schemars`.)* |
| 7 | Nombres de evento/comando divergentes (`bundle:changed` vs `bundle://changed`, `query` vs `query_bundle`…) | Registro de constantes compartido + `ipc.ts` generado + smoke test que abre bundle, edita y asserta snapshot poblado. |
| 8 | Doble escritor de SQLite (comando + watcher) | **Un watcher = único escritor.** Comandos solo escriben el `.md`. |
| 9 | `RelPath` newtype vs `type RelPath = String` | Newtype validado en todas partes (es el chokepoint de path-traversal). |
| 10 | DDL del grafo definido por dos arquitectos; `checks` vs `diagnostics` | `store` es dueño único del DDL; ORPHAN/LINK-STUB **sintetizados** (no materializados); columnas casan con los nombres del `Check`. |
| 11 | `body:` subcadena (proto) vs FTS MATCH (token) | Subcadena en todas las fachadas; FTS solo como acelerador superset. Un solo `match_token`. |
| 12 | Generadores puros vs que escriben | Puros (devuelven `Mutation`); la workspace aplica y diffea para `{written,removed,unchanged}`. |
| 13 | `merge_frontmatter` (patch merge-patch RFC 7386: `null` elimina la clave) no existía en el core | Vive en el **core** (es lógica OKF), no en el MCP. |
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

> **⚠️ TABLA MAYORITARIAMENTE DE ÉPOCA (v0.2).** `CLAUDE.md` la cita junto a `§10` como autoridad
> para resolver contradicciones ya zanjadas, así que conviene saber qué queda en pie. Se escribió
> para un producto con GUI, git y OKF, y el giro headless (`§19`) más la migración a Markdown
> universal (`§20`) se llevaron por delante la mayoría de sus filas.
>
> **Siguen vigentes**: *Errores* (taxonomía y código estable cruzando `CoreError`→`ErrorCode`→exit
> code; la supervisión del watcher), *Un bundle por proceso* (el lockfile existe, y `E23-H23` lo hizo
> reclamable por TTL+PID), y la parte de *Seguridad* sobre escapado de FTS5 y path-traversal
> (`RelPath`, invariante #6).
>
> **De época, NO vigentes**: *Migración del prototipo* (`lodestar import` se borró en `E15-H03`, y no
> hay `git init` que hacer), *Versionado OKF* (`okf_version` dejó de tener semántica en `§20`),
> *Packaging* en su parte Tauri, *Testing/paridad* en sus partes de test diferencial (retirado en
> `E15-H04`), store Svelte y e2e de Tauri, *First-run* (`lodestar init` retirado en `E15-H03`; no se
> hace scaffold de nada), y **las tres filas de git** —*Sincronización/remoto*, *Paridad con `git`
> CLI* y el shell-out de *Seguridad*—, porque `lodestar-vcs` y `git2` **se borraron del repo** en
> `E15-H01` (`§20.13`).
>
> **Corregida por historia posterior**: la fila *Config* dice que la configuración por bundle es
> `lodestar.toml`; es **`.lodestar/config.yaml`** desde `E15-H08`, y es **opcional** — un directorio
> con `.md` sueltos y sin `.lodestar/` es un workspace válido (`§20.1`). Un lector que siga esta fila
> buscará un fichero que no existe.

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
> **Ratificado 2026-07-22** (puerta 1 de `/planificar`; fuente: `docs/history/REFACTOR.md`; propuesta:
> `docs/history/REFACTOR_DISENO_PROPUESTA.md`). Lodestar deja de posicionarse como "editor local-first con git
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
                       lodestar-app   (servicios de caso de uso · mapa de códigos de error)
                          ▲       ▲
                   lodestar-mcp · lodestar-cli     (las dos fachadas del motor headless: 5–15 líneas, CERO dominio)
                   (src-tauri RETIRADO de `main` → rama experimental/ui-desktop)
```

- **`core::schema`** (nuevo módulo, **PURO**): tipo `Schema` (catálogo de `DocType`, campos,
  `requiredFields`, `allowedStatuses`, typed relations, lifecycle, plantillas) + funciones de validación
  que, dado un `Schema` + un `Bundle`, producen `Vec<Check>` (extiende `conform`). La **aplicación de
  plantillas** es generación pura (como `gen_index`/`gen_tag_indexes`). **Leer** `.lodestar/schema.yaml` /
  `.lodestar/templates/` es I/O de `workspace` (patrón `Config::load`); el core nunca abre ficheros.
- **`lodestar-app`**: ensambla `ChangeSet`, conduce plan→validar→aplicar→verificar y mapea
  `CoreError`/`WorkspaceError` → **códigos de error** estables. (**Actualización E29-H11,
  `decisiones §16(b)`**: el crate construyó también un **envelope** de protocolo — decisión D3, «el
  envelope es framing, no dominio» — pero ninguna fachada llegó a ensamblarlo nunca; el wire real es
  `structuredContent`/exit codes directos, §20.10. Se retiró por no tener consumidor.)

### 19.3 Tipos nuevos (invariante #4: UNA vez en `core::types`)

Se congelan en `lodestar-core::types`:

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
  `transition_status`/`apply_fix`); reutiliza `FrontmatterPatch` (merge-patch RFC 7386: una clave
  presente con valor `null` **elimina** esa clave, que es lo que el propio RFC define — no una
  capacidad adicional de distinguir «asignar null» de «eliminar»).
  **Hoy son SIETE** (§20.11): `add_relation`/`remove_relation`/`transition_status` cayeron con el
  modelo universal (`E21-H01`) y `apply_fix` en `E23-H11` (sin productor de `Fix` desde que
  `E20-H03` retiró `core::schema`, fallaba siempre). Las 11 de esta línea son el diseño de época.
- `RiskAssessment { level, reasons }` — lógica pura nueva alimentada por backlinks/blast-radius.
- `SemanticDiff` — **reutiliza `OkfDiff`** (`core::diff`, port de `diffSnap`), ampliado con
  `diagnosticsIntroduced`/`diagnosticsResolved`.
- `ValidationReport { valid, summary{errors,warnings,info}, diagnostics: Vec<Check> }` — sobre
  `Analysis.hard_fail`/`warn_count`. (El campo se llamaba `conformant`; `E23-H14` cerró el adjetivo
  de la pareja `Conformance / Conformant` de §20.3, igual que `NONCONFORMANT_RESULT` abajo.)
- `ChangeReceipt { id, change_set_id, previous_revision, result_revision, changed_paths, semantic_diff }`.
- **Códigos de error** (§13 de REFACTOR): enum estable en `core::types` (patrón `CheckCode`, wire por
  `#[serde(rename)]`): `WORKSPACE_NOT_FOUND`, `WORKSPACE_RECOVERY_REQUIRED`, `CONCEPT_NOT_FOUND`,
  `AMBIGUOUS_REFERENCE`, `REVISION_CONFLICT`, `PLAN_STALE`, `PLAN_EXPIRED`, `PERMISSION_DENIED`,
  `INVALID_SCHEMA`, `INVALID_RESULT`, `INBOUND_LINKS_EXIST`, `RELATION_CONSTRAINT_VIOLATION`,
  `WRITE_CONFLICT`, `RESULT_TOO_LARGE`, `RECOVERY_FAILED`, `INTERNAL_IO_ERROR`.
  (`NONCONFORMANT_RESULT` → `INVALID_RESULT` en `E23-H14`, la única vez que se abre este catálogo:
  ver `§20.3`. `CONCEPT_NOT_FOUND` pasó a `DOCUMENT_NOT_FOUND` en `E16-H06`. Siguen siendo 16.)

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

> **Actualización E15-H08**: `lodestar.toml` **ya no existe** (borrado; cierra `decisiones §8`),
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
`schema_inspect` (→ `metadata_inspect` en §20.10) · `graph_query` · `impact_analyze`; **VERIFY** `knowledge_check`; **CHANGE**
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
llamada (seguridad §19.7). **Transporte** (decisión D6b): se mantiene **stdio** (decisiones §3) y se
activa **`outputSchema`** derivado con la feature `schemars` (ya preparada, §10 fila 14); `rmcp`
**diferido**. `contracts/mcp.yml` se **reescribe** 13→10 y el guardián de contrato lo vigila.

> **Actualización E29-H11** (`decisiones §16(b)`): esta sección describía también un «envelope
> común» — `{ ok, workspaceRevision, summary, data, diagnostics, warnings, resourceLinks }` — como
> forma de respuesta compartida por las diez tools. Se construyó (`lodestar-app`, E10-H01) pero
> ninguna fachada llegó a ensamblarlo jamás: el wire real que sirve cada tool (§20.10,
> `contracts/mcp.yml`) es su tipo de servicio propio en `structuredContent`, sin envolver, y la CLI
> responde con exit codes. Se retiró del crate por no tener consumidor.

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
| **E10** — Esquemas + lectura headless | 1 | `core::schema` puro; `ConceptRevision`/`WorkspaceRevision`/`ConceptRef`; extensión de `Check`; envelope + códigos de error; crate `lodestar-app`; `workspace_status`/`knowledge_search`/`knowledge_get`/`schema_inspect` (→ `metadata_inspect` en E20-H03)/`knowledge_check` |
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

> **La pareja `Conformance / Conformant` está completa desde `E23-H14`** (2026-07-25). El sustantivo
> se sustituyó en E16 (`ApplyConformance` → `ApplyValidation`, `Store::conformance_counts` →
> `validation_counts`), pero el **adjetivo** sobrevivió tres épicas más en la superficie activa
> —`conformant`, `requireConformantResult`, `allowNonconformant` y el código de error
> `NONCONFORMANT_RESULT`—, porque completarlo obligaba a **abrir el catálogo de 16 códigos** de
> `§19.3`, declarado congelado. Era el **único** de los 29 criterios de aceptación de
> `REFACTOR_PHASE_2` demostrablemente incumplido («no existe terminología OKF en la API pública»).
> Se cerró aprovechando que v0.3.0 ya es incompatible con v0.2.x: romper el wire costaba cero
> entonces y dejaba de costarlo en cuanto se publicara. Ver `decisiones §13`.
>
> Wire resultante: `conformant` → `valid` · `requireConformantResult` → `requireValidResult` ·
> `allowNonconformant` → `allowInvalid` · `NONCONFORMANT_RESULT` → `INVALID_RESULT`.

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

**BOM UTF-8** (E24-H01): un `U+FEFF` al frente del fichero **no oculta el bloque** de frontmatter, y
se **conserva byte a byte** en toda reescritura — nunca se normaliza al leer de disco, porque
`workspace_revision` hashea los bytes crudos y strippearlo al leer sin reemitirlo declararía un
cambio espurio en cada round-trip. No pertenece ni al frontmatter ni al cuerpo: `SplitFront::body`
lo deja fuera, y quien traduzca un offset del cuerpo a una posición del fichero debe usar
`SplitFront::body_offset`. Su falta de portabilidad se avisa con `DOC-BOM` (§20.9).

**Título derivado** — `frontmatter.title` → primer heading H1 → nombre del fichero. Es **solo una
heurística de presentación**: `title` no se convierte en propiedad reservada.

**Edición de frontmatter** — la operación genérica es `patch_frontmatter`, que modifica solo las
claves pedidas, preserva las demás, no reordena innecesariamente y mantiene el cuerpo intacto. Su
semántica es **merge-patch RFC 7386**, sin más (corregido en E30-H03, `decisiones §23/D-02` —
criterio ratificado el 2026-08-06):

- una clave **ausente** del patch no se toca;
- una clave presente con un valor **se escribe o reemplaza** con él;
- una clave presente con valor **`null` se ELIMINA**. Eso es exactamente lo que RFC 7386 define, no
  una capacidad adicional del motor.

Corolario, y es el punto que este párrafo afirmaba al revés hasta v0.5.0: **no existe forma, por
esta vía, de asignar literalmente `null` a una clave de primer nivel** — el wire no puede expresar
esa distinción porque el `null` es el sentinel de borrado. Verificado ejecutando: `{"status":"~"}` y
`{"status":"null"}` escriben el *string* `'~'`/`'null'`, no el null YAML. El `Some(Null)` que el
tipo del core (`FrontmatterPatch`) sí sabría representar es **inalcanzable desde
`patch_frontmatter`**, que serializa como merge-patch.

Matiz verificado, porque el borrado **no** es uniforme por profundidad: el `null` es sentinel de
borrado **solo en el primer nivel del patch**. Anidado dentro de un mapa o de una lista del valor
(`{"meta": {"a": null}}`, `{"tags": [null, 1]}`) sobrevive como null YAML literal, porque a partir
del primer nivel el valor se escribe tal cual en vez de recorrerse como patch. Un `null` ya presente
en el fichero también se preserva si el patch no nombra su clave.

Ampliar el wire para expresar «asignar `null`» queda **fuera de alcance** salvo que aparezca un caso
real que lo pida (`decisiones §23`): quien necesite ese valor debe pasarlo como cualquier otro
escalar, y el propio RFC no lo permite. El plan debe declarar si el bloque se reserializará entero.

> **E31-H02 (`decisiones §26`)**: reescribir el **cuerpo** ya no reserializa nunca la cabecera. El
> brazo `ReplaceBody` —al que normalizan `replace_text`, `edit_section`, `replace_body`,
> `delete remove_links` y el `rewriteInboundLinks` de `move`— es hoy un patch quirúrgico
> (`model::replace_body_preservando_cabecera`), inverso exacto de `SplitFront::body`: la cabecera
> sobrevive byte a byte, incluido su separador y **el bloque cuyo YAML no se deja leer**, que hasta
> v0.5.0 se borraba en silencio. El único camino que aún puede reserializar es la edición del propio
> frontmatter (`patch_frontmatter`), y solo cuando el patch quirúrgico no alcanza: eso es lo que
> declara `PatchedDocument.reserialized`. Una operación cuyo resultado es idéntico al documento de
> partida se anota en `PlanResult.noOpOperations` en vez de desaparecer del plan.

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
    Document(RelPath),        // otro .md del INVENTARIO → arista del grafo
    WorkspaceFile(RelPath),   // fichero del proyecto que NO es un documento del inventario: existe, pero NO es nodo
    ExternalUri(String), SelfAnchor(String), Missing(RelPath),
    WorkspaceDirectory(Option<RelPath>),   // E23-H11; None = la raíz
    EscapesWorkspace,
}
```

> **Dos precisiones decididas en E17-H02**, que `REFACTOR_PHASE_2 §Fase 7` no cubre:
>
> 1. **Href raíz-absoluto (`/beta.md`)**: se resuelve **relativo a la raíz del workspace**. Es
>    determinista y sin heurística (que es lo que prohíbe `§20.7`), coincide con cómo renderizan los
>    `.md` de un repo GitHub/GitLab, y el invariante *"ningún path público es absoluto"* de `§20.2`
>    habla de las **rutas que Lodestar expone**, no de los hrefs que escribe el usuario en su
>    contenido. La alternativa (`EscapesWorkspace`) rechazaría un patrón real y frecuente.
> 2b. **Navegación pura** (E17-H03; **resuelto en E23-H11**): un href que **no nombra ningún
>    segmento propio** —`.`, `./`, `..`, `../`, `../..`— designa un *directorio* (el del propio
>    documento o uno por encima), no un fichero. Como `§20.6` prohíbe convertir un directorio en su
>    `index.md`, hasta E23-H11 se clasificaba **`EscapesWorkspace`** por no haber variante mejor, y
>    eso lo hacía `Err`: un `[volver](../)` —que GitHub renderiza sin problema— **tumbaba la puerta
>    de CI**. E17-H03 ya dejó escrito que el arreglo correcto era ampliar el enum, no parchear el
>    diagnóstico, y así se hizo: entra **`WorkspaceDirectory(Option<RelPath>)`** (`None` = la raíz,
>    que no es nombrable como `RelPath`). No es nodo del grafo, no produce diagnóstico, y
>    `EscapesWorkspace` queda para lo que de verdad sale **por encima** de la raíz —que sigue siendo
>    `Err`, porque es el chokepoint semántico de contención—. Un href que **sí** nombra algo sigue
>    teniendo path aunque también sea un directorio: `[x](guias/)` es `Missing("guias")`, nunca
>    `guias/index.md` (no se introduce heurística de barra final).
>    `move_document` recalcula también estos destinos (E23-H11): si el documento cambia de
>    profundidad, un `../` pasaría a señalar otro directorio en silencio.
>
> 2. **Un `.md` que existe en disco pero está EXCLUIDO del descubrimiento** es **`WorkspaceFile`, no
>    `Missing`**. Por eso la variante se define por «no es un documento del inventario» y no por «no
>    es `.md`»: decir `Missing` de un fichero que está ahí sería mentir sobre el disco y produciría
>    un `LINK-TARGET-MISSING` espurio sobre un enlace que el usuario ve funcionar. Consecuencia para
>    quien construye el `Inventory`: los `.md` excluidos van en `other_files`, no en `documents`.
>
>    **Límite conocido y aceptado** (E17, cableado de `other_files`): esto solo alcanza a los
>    ficheros que el walker **visita**. Los que quedan **podados** —por `discovery.exclude`
>    (`.git/**`, `.lodestar/**`) o por un `.gitignore`/`.lodestarignore` del árbol— nunca se
>    visitan, así que un enlace a ellos sigue siendo `Missing`. Cubrirlo exigiría **dejar de podar
>    directorios ignorados**, es decir recorrer `.git/` y `node_modules/` enteros en cada análisis:
>    una regresión directa del descubrimiento de E15 a cambio de un caso marginal. Se acepta el
>    coste. (El ejemplo que esta precisión usaba antes, `vendor/dep.md` bajo `.gitignore`, cae
>    justo en el lado podado — por eso se ha retirado del texto.)

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
    pub diagnostics: BTreeMap<RelPath, Vec<Check>>,   // `Check` es el tipo de `§4.1` (invariante #4)
}

impl Analysis {                    // E17-H04: derivados, NO campos
    pub fn hard_fail(&self) -> usize;   // nº de FICHEROS con algún `Err` (`§10` fila 4)
    pub fn warn_count(&self) -> usize;  // nº de DIAGNÓSTICOS `Warn`
}
```

Un **documento aislado no es inválido**: es una propiedad consultable (`graph.isolated = true`) que
no genera warning por defecto.

Precisiones de E17-H04 (fase verde):

- **`hard_fail`/`warn_count` son métodos derivados de `diagnostics`, no campos**: la lista de seis
  campos de arriba es literal, y la puerta de CI (`WorkspaceConfig::gate_blocked`,
  `workspace_status`, `knowledge_check`) los sigue consumiendo con la misma semántica. Así **no
  puede existir** un contador desincronizado de la lista de la que sale (invariante #3).
- **`outgoing` lleva TODOS los enlaces**, no solo las aristas: los externos, los anchors propios y
  los que apuntan a ficheros del proyecto viajan también, porque los necesitan
  `knowledge_get.outgoingLinks`, `move_document` y la tabla `links` del store v2 (`§20.12`). El
  **grafo filtra**: nodo solo lo que es documento, arista solo `Document`/`Missing`
  (`LinkTarget::is_internal`, la única definición de «enlace interno»).
- **Precisión (E17, cableado de `other_files`): un `Missing` solo es fantasma si su destino sería un
  documento Markdown** (`RelPath::is_markdown`). Los nodos son «todos los documentos **Markdown**
  descubiertos», así que un enlace roto a código (`[x](src/no_existe.rs)`) **no** mete un vértice
  `.rs` en el grafo: sigue siendo un colgante de `Analysis::dangling` con su `LINK-TARGET-MISSING`
  (`warn`), pero no es un nodo. El filtro se aplica **solo** a `Missing`: un destino `Document` está
  en el inventario y es un documento aunque `discovery.include` admita otra extensión — decidirlo
  por el nombre sería la clasificación por extensión que `§20.6` prohíbe. Es el mismo discriminador
  que ya decidía la severidad de `LINK-TARGET-MISSING`, y por eso se comparte: si divergieran,
  habría nodos del grafo que la conformidad no considera documentos.
- **`incoming` es literalmente la inversa**: una entrada por **enlace** (un origen que enlaza dos
  veces aparece dos veces) y el `ResolvedLink` que lleva es *el mismo* que su origen tiene en
  `outgoing`. La deduplicación por vecino vive en el grafo, no en la lista.
- **Aislado** = sin enlaces internos entrantes ni salientes. `Missing` **cuenta** como enlace
  interno: enlazar a un fantasma es participar en el grafo.

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
| `DOC-BOM` | BOM UTF-8 al frente del documento: se interpreta y se conserva byte a byte, pero no es portable (aviso, E24-H02) |

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

> **Actualización E23-H11.** `knowledge_search` gana `include: ["frontmatter.<fieldPath>"]`: la
> **proyección** de los campos de frontmatter que pida el llamador sobre cada resultado (valores YAML
> crudos, campo ausente = clave ausente). Sin ella, ver el `status` de 30 resultados costaba 30
> `knowledge_get` — el N+1 que dejó E19-H05 al retirar los campos privilegiados OKF sin sustituto
> genérico. Una entrada mal formada es `INVALID_SCHEMA`, no un descarte silencioso.
> En la misma historia **desaparece `sort`**, que se aceptaba y se ignoraba desde E10-H09: el orden
> determinista (score desc, path asc) es el único, y es la base del cursor-offset.
> Y `workspace_status` gana `receipts`, el listado acotado de recibos persistidos (mtime desc), para
> que perder el `receiptId` de un `change_apply` no deje el undo inalcanzable. La superficie sigue
> siendo de **diez** tools: listar recibos no justifica una undécima.

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

> **Actualización E23-H11: son SIETE.** `apply_fix` se **retira** de la superficie (del enum de
> `change_plan`, de las ops admitidas en masa, de `NormalizedOperation` y de `plan.rs`, junto con
> `CoreError::FixNotFound`). Motivo: E20-H03 retiró `core::schema` y con él el único productor de
> `Fix`, así que la operación fallaba **siempre** —y con un código que apuntaba al sitio equivocado
> (`DOCUMENT_NOT_FOUND` sobre un documento que existe)—, mientras se anunciaba en el `inputSchema`
> como una de las ocho. Hoy es una op desconocida → `INVALID_SCHEMA`. El lado de **lectura** de los
> arreglos (`Fix`, `Check.fixes`, `knowledge_check.includeSuggestedFixes`) **no se toca**: un array
> vacío se lee como «no hay sugerencias» y eso es verdad. El análisis de qué haría falta para que la
> capacidad vuelva está en `docs/history/PROPUESTA_FIXES.md`.

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

---

## 21. Superficie externa y distribución (E27)

> **Ratificada el 2026-08-01** (puerta 1 de la épica E27, `decisiones §17`). **Aditiva: no
> supersede nada.** Regula lo que el proyecto muestra hacia fuera —README, documentación de usuario,
> demo, pipeline de release y embudo de contribución—, no el motor. Contexto: una review OSS externa
> (evaluada y verificada punto a punto, 2026-08-01) concluyó que el proyecto está **por delante de su
> adopción**: la deuda dominante de v0.5.0 no es de motor, es de producto/distribución/comunidad.

### 21.1 Regla de idioma (amplía el invariante de `requirements/README.md`)

- **Superficie pública, en inglés**: `README.md`, `docs/user/`, `examples/demo/`, `CONTRIBUTING.md`,
  `SECURITY.md`, `CODE_OF_CONDUCT.md` y los templates de `.github/`.
- **Interno, sigue en español**: `ARCHITECTURE.md`, `decisiones/`, `requirements/`, `docs/` (specs
  y workflows), `contracts/`, código, comentarios, mensajes de error del wire y commits.
- La frontera es de **audiencia**, no de directorio: lo que un adoptante lee antes de decidir se
  escribe en inglés; lo que gobierna el desarrollo del repo se queda en español.

### 21.2 Distribución

- **Vías soportadas**: binarios precompilados de GitHub Releases (3 plataformas) y
  `cargo install --git`. **crates.io queda diferido** (`decisiones §17`, reabrible): publicar es
  permanente y los crates de dominio no están pensados como API de librería estable.
- **Guardarraíles del pipeline** (`.github/workflows/release.yml`, E27-H01):
  1. un step temprano **falla si `github.ref_name != "v" + workspace.package.version`** — convierte
     en error de CI la clase de fallo del tag `0.5.0` sin prefijo (y el caso inverso: tag empujado
     sin subir la versión);
  2. cada plataforma publica un **`SHA256SUMS-<target>.txt`** como asset del release.
- **Firma/notarización: diferida**, no descartada (`RELEASING.md`); los checksums cubren la parte
  barata de la integridad.

### 21.3 Taxonomía documental y demo ejecutable

- `docs/user/` — documentación de **usuario**, en inglés.
- `docs/` raíz — **specs vigentes e internas**, en español: `REFACTOR_PHASE_2.md` (spec de
  comportamiento viva, citada por ~51 ficheros del repo: **no se mueve** — el criterio taxonómico es
  *vigente/superseded*, no *viejo/nuevo*) y `WORKFLOWS.md`.
- `docs/history/` — propuestas y specs **superseded** (`REFACTOR.md`, `REFACTOR_DISENO_PROPUESTA.md`,
  `PROPUESTA_CLI.md`, `PROPUESTA_FIXES.md`), conservadas como historia del proyecto.
- `examples/demo/` — un workspace Markdown pequeño con defectos **deliberados** (un enlace roto, un
  huérfano) que sirve de **documentación ejecutable**: el quickstart del README y `docs/user/` se
  escriben contra él, y un **job de smoke en CI** ejecuta su guion y aserta las salidas clave, para
  que README y demo no puedan pudrirse en silencio.

### 21.4 Contribución y seguridad

- **Issues-first**: bugs y docs se aceptan por PR directo (con checklist); las features requieren
  issue previa donde el mantenedor decide si pasan por el proceso de diseño del repo. El proceso SDD
  interno (historias ratificadas, jueces ciegos) no se les exige a contribuidores externos: lo aplica
  el mantenedor al integrar. **Discussions: OFF** por ahora (revisar cuando haya tráfico).
- **Código de conducta**: Contributor Covenant 2.1, en inglés; contacto `dbareagimeno@icloud.com`.
- **Seguridad**: GitHub **Private Vulnerability Reporting** como canal primario + el email como
  fallback. El alcance declarado es honesto con el producto: motor local por stdio, sin red — la
  superficie de ataque relevante es parsing y path-traversal (chokepoint `RelPath`, invariante #6).

### 21.5 Regla transversal de honestidad (ligada a `decisiones §14`)

**Mientras `decisiones §14` siga abierta** (el store SQLite no tiene consumidor y el watcher no
corre en el motor), la superficie externa **no presenta `reindex`/la cache como camino de lectura del
producto ni promete rendimiento a escala**. La cache se describe como lo que es: derivada y
reconstruible. Es el principio de E23 («la documentación no afirma nada falso») aplicado donde el
coste de una afirmación falsa es mayor.

**Principio rector de E27**: *la superficie externa solo promete lo que el motor ejecuta hoy.*

---

## 22. Banco de pruebas y gate de rendimiento (épica de evidencia)

> **Ratificada el 2026-08-10** (puerta de diseño de la épica de evidencia; la ejecuta `E33`,
> `requirements/epica-33-banco-evidencia.md`). Cierra el diseño de `decisiones §9` punto 1 (gate de
> rendimiento, condición de entrada de `decisiones §14`) y convierte el testbench de `decisiones §23`
> (189 casos contra el homelab, `docs/qa/testbench/`) en **banco permanente por release**. Esta
> sección es **instrumento interno**: mientras `decisiones §14` siga abierta, `§21.5` sigue vigente y
> **nada de lo que el banco mida se promete en la superficie externa**. La épica que la ejecuta
> produce datos y análisis; **no cierra** `decisiones §14`, `§22` ni `§24` — esas decisiones son del
> usuario. (Ojo con la numeración: en esta sección, `decisiones §N` es siempre una ficha de
> `decisiones/`, no una sección de este documento.)

### 22.1 Las dos piezas del banco

El banco separa dos preocupaciones de vida distinta, con el arnés JSON-RPC/stdio
(`docs/qa/testbench/lodestar_harness.py`) como ejecutor común de todo lo que toca el wire:

1. **Banco de conformidad** (python, `docs/qa/testbench/`): casos esperado-vs-real **asertables**
   contra el contrato. Hereda el arnés de `decisiones §23` (sesiones por lote, worktrees efímeros,
   placeholders `@stepN`, distinción error-de-tool vs error-de-protocolo) y le añade lo que le
   faltaba para ser permanente: veredicto mecánico, portabilidad y un corpus propio (§22.2, §22.3).
2. **Banco de rendimiento** (Rust, crate interno `lodestar-bench`, `publish = false`): mide los
   servicios de `App` —el patrón de E14-H05— más una calibración sobre el wire MCP real vía el
   arnés python (§22.4, §22.5).

### 22.2 Corpus canónico y generador de escala

- **El homelab deja de ser el campo de pruebas del gate**: es privado, mutable y no-CI. Queda como
  corrida **opcional de dogfooding** (el modo `--root` del arnés se conserva).
- **Corpus canónico de conformidad**: generado **determinísticamente** (script versionado, semilla
  fija) — ~50–100 documentos con la fauna completa: grafo de enlaces con huérfanos y dangling,
  frontmatter heterogéneo y consultable, y los sets patológicos de `make_fixtures.py` integrados.
  Los esperados del banco se escriben contra él, así son estables.
- **Generador de escala**: el de E14-H05 (`crates/lodestar-app/tests/escala.rs`) se extrae a código
  compartido y gana dos perfiles — **plano** (10k homogéneo, comparable con las cifras históricas de
  E14-H05) y **realista** (distribución de enlaces y frontmatter modelada sobre corpus reales) — y
  tres escalas (~100 / ~1k / ~10k).
  - *Precisión de implementación (E33-H01, no cambia el diseño)*: el perfil realista entregado usa
    una distribución **sintética y uniforme** (PRNG determinista), **no** calibrada contra corpus
    reales. Cumple el propósito —que no se mida solo sobre el corpus plano: hay enlaces que
    resolver, backlinks que computar y frontmatter heterogéneo que consultar—, pero no reproduce la
    cola larga típica de un corpus humano. Si alguna medición llegara a depender de esa forma,
    habría que calibrarla primero.
- Regla heredada del repo: las fixtures grandes **se generan en runtime** (tempdir), nunca se
  commitean.

### 22.3 Esperados asertables y veredicto mecánico

- El formato de lote gana un campo `expect` evaluable por el runner (código de error esperado,
  subcampos del `structuredContent`, invariantes como «`workspaceRevision` intacta»). El runner
  emite PASS/FAIL por caso y un resumen agregado; **cero veredictos manuales** en la corrida por
  release (la verificación adversarial humana queda para campañas exploratorias).
- **Criterio de selección**: entra al banco lo que asevera **contrato estable** (códigos,
  invariantes, formas de respuesta), no lo que asevera contenido de un corpus concreto. Base: los
  `verify_*` de `decisiones §23`, los invariantes transversales verificados conformes (informe
  2026-08-06 §5) y una muestra por lote temático.
- **Centinelas de decisiones abiertas** (`decisiones §22` y `§24`): casos cuyo esperado es el
  **comportamiento vigente**, citando la ficha abierta. Si el comportamiento cambia, el centinela
  falla y obliga a actualizar esperado y ficha a la vez. El banco **detecta** el cambio; **no lo
  juzga** ni cierra la ficha.

### 22.4 Métricas, umbrales y baseline (el gate de `decisiones §9` punto 1)

- **Qué se mide**: cold-open (`App::open` + primera llamada) y **coste por llamada** —el sustituto
  ratificado del histórico «edit→UI < 150 ms», que murió con la UI— como p50/p95 de N iteraciones
  por tool de lectura y por el ciclo `change_plan`→`change_apply`, más el tamaño de payload (proxy
  de tokens), a las tres escalas y en las tres variantes de camino de lectura (§22.5).
- **Umbral-tras-medición**: los umbrales **no se inventan a priori** — la primera corrida produce
  los números y el usuario los ratifica con ellos delante (puerta interna de la épica). Anclas de
  esa conversación: **p95 ≤ 1 s por tool de lectura a 10k** y **cold-open ≤ 5 s**. Ratificados, el
  gate se codifica y falla (exit ≠ 0) si se violan.
- **Baseline por máquina**: los umbrales absolutos solo se juzgan en la máquina donde se ratificó
  la baseline (la de release); cada release registra su corrida para tendencia. En CI solo corre un
  **smoke barato** (escala mínima, sin umbral absoluto) que garantiza que los artefactos del banco
  compilan y corren — que el banco no se pudra en silencio.

### 22.5 Medir «con cache» sin conectarla

El producto no lee el store (`decisiones §14`); el banco lo mide **sin tocar el camino de lectura**,
por la API pública existente, en tres variantes:

1. **Disco-reparseo** — el producto actual (`Workspace::document_set()` → `discovery::discover` en
   cada llamada).
2. **SQLite-raw** — `Store::rebuild()` + `Store::document_set()` (`DocumentSet::from_store`).
   **Advertencia que la evidencia debe rotular**: `from_store` reconstruye el `FileMap` desde los
   `raw` en SQL y **re-parsea**; esta variante mide la cache *tal como está construida* (ahorra
   walk+IO, no parse). Se registra también el coste del `rebuild` (el precio de tener la cache).
3. **RAM-memoizado** — un `DocumentSet` construido una vez y reutilizado entre llamadas: la **cota
   superior** de lo que cualquier cache puede dar, y una alternativa que `decisiones §14` debe ver.

Ninguna variante conecta el store al producto ni roza el invariante #3: son caminos de **medición**
dentro del bench. El paquete de evidencia inventaría además el coste de conexión ya conocido
(walker del store sin `DiscoveryPolicy`, divergencia `field_path` core↔store —`§16(l)`—, y el
destino del watcher —`§16(c)`—) como parte del precio de la opción (a) de `decisiones §14`.

### 22.6 Enganche a release

El banco corre **por release**: `RELEASING.md` gana un paso (lo escribe la historia que entrega el
enganche, no antes de que la herramienta exista) — correr conformidad + rendimiento contra el
binario release en la máquina de la baseline y **commitear la corrida datada** en `docs/qa/`. Eso
es, literalmente, «banco permanente por release». Opcionalmente, un `workflow_dispatch` lo dispara
a demanda (conformidad + smoke, sin juzgar umbrales absolutos en runners compartidos).

### 22.7 Dogfooding acotado con registro

La otra mitad del dato de `decisiones §14`: ¿el reparseo por llamada **molesta** en uso real
(~100 docs) o solo en el arnés sintético (10k)? Protocolo: (1) el propio repo (`decisiones/` +
`requirements/`) se usa como workspace lodestar vía MCP en las sesiones de trabajo — las consultas
que `decisiones/README.md` ya documenta, ejecutadas de verdad; (2) diario de fricciones en
`docs/qa/` (fecha · tool · fricción · latencia percibida); (3) el número frío que acompaña a la
percepción: la corrida del banco a escala «repo real»; (4) **ventana acotada** — el dogfooding
computa para `decisiones §14` hasta que el paquete de evidencia se cierra; lo posterior alimenta
fichas nuevas, no bloquea el cierre.

### 22.8 El paquete de evidencia para `decisiones §14`

Un documento único y datado en `docs/qa/` con: la tabla de mediciones (variantes × escalas ×
tools), el dato de dogfooding, el inventario del coste de conexión (§22.5), y el análisis de las
tres salidas de la ficha —conectar / acotar / retirar— **contra los datos**, actualizando o
refutando la recomendación escrita en ella. Termina en **«lista para decidir»**: la decisión, su
ratificación y el cambio de estado de `decisiones §14` son del usuario, fuera de la épica.
