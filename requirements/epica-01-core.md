# E1 — `lodestar-core` puro

> **Fase**: `§14.1`. **Objetivo de la épica**: portar TODA la lógica OKF del prototipo a un crate
> Rust **puro** (`#![forbid(unsafe_code)]`, sin I/O/DB/git/runtime), congelar el contrato de tipos
> de `§4.1`/`§4.4`, e instalar el **arnés diferencial JS-vs-Rust** como red de seguridad.
> Al cerrar E1, la semántica OKF es testeable sin GUI/DB/runtime.
> Referencias maestras: `ARCHITECTURE.md §4`, `§10` filas 3/4/5/11/12/13, `§12` (i18n keyed).

**Principio rector de la épica**: *port 1:1 del prototipo, quirks incluidos*. Para cada historia,
abre la función JS citada en `prototype/index.html` y reproduce su semántica exacta. El arnés
diferencial (E1-H18) es el juez.

---

### E1-H01 — `RelPath`: newtype validado (chokepoint de path-traversal)
- **Objetivo**: el tipo `RelPath(String)` con validación que rechaza absolutas y `..` y normaliza.
- **Referencias**: `ARCHITECTURE.md §4.1`, `§10` fila 9 · `CLAUDE.md` invariante 6.
- **Alcance**:
  - `pub struct RelPath(String)` con `RelPath::new(&str) -> Result<Self, CoreError>` que: rechaza
    rutas absolutas, cualquier componente `..`, normaliza separadores a `/`, colapsa `.` y `//`.
  - `Display`, `AsRef<str>`, `Ord`/`PartialOrd` (orden lexicográfico estable para `BTreeMap`/`BTreeSet`),
    `Serialize`/`Deserialize` (deserializa **validando**, no por `From<String>` crudo).
  - Helpers de modelo asociados o libres: `basename`, `dir_of`, `concept_id` (port de
    `basename`/`dirOf`/`conceptId` del prototipo).
- **Fuera de alcance**: I/O de rutas (no toca el FS).
- **Criterios de aceptación**:
  - `RelPath::new("/etc/passwd")`, `RelPath::new("../x")`, `RelPath::new("a/../../b")` → `Err`.
  - `RelPath::new("a//b/./c.md")` normaliza a `a/b/c.md`.
  - `type RelPath = String` **no** aparece en ningún sitio (grep en CI).
  - Deserializar un JSON con un path inválido falla (no crea un `RelPath` inseguro).
- **Dependencias**: E0-H01.
- **Pruebas**: tabla de casos válidos/inválidos; round-trip serde; property test (cualquier `RelPath` válido no contiene `..`).

### E1-H02 — `CoreError` y taxonomía de errores del core
- **Objetivo**: el enum de errores puro del core, base de la taxonomía del `§12`.
- **Referencias**: `ARCHITECTURE.md §6` (WorkspaceError envuelve CoreError), `§12` (errores).
- **Alcance**:
  - `pub enum CoreError` con variantes para: path inválido, YAML malformado (con detalle), guarda de
    tamaño excedida (diff/LCS), export I/O (genérico `Write`), etc. `thiserror` con mensajes en español.
  - **No** mete variantes de DB/git/runtime (esas viven en `store`/`vcs`/`workspace`).
- **Criterios de aceptación**: `CoreError: std::error::Error + Send + Sync`; cada variante tiene mensaje legible.
- **Dependencias**: E0-H02.
- **Pruebas**: snapshot de los `Display` de cada variante.

### E1-H03 — Contrato de tipos de conformidad: `Severity` · `CheckCode` · `Check`
- **Objetivo**: congelar `Severity`, los 15 `CheckCode` y `Check` exactamente como `§4.1`.
- **Referencias**: `ARCHITECTURE.md §4.1`, `§10` filas 3/4 · prototipo `chk`, `validateFile`.
- **Alcance**:
  - `Severity { Pass, Info, Warn, Err }` con `Ord` **deliberado** (`Err` máximo) y `#[serde(rename_all="lowercase")]`.
  - `CheckCode` con **una sola enum** y `#[serde(rename="OKF-FM01")]` etc. para los 15 códigos del `§4.1`
    (incluido `OKF-CONFLICT`). El valor de wire ES la cadena con guion.
  - `Check { level: Severity, code: CheckCode, msg: String, targets: Vec<RelPath> }` —
    nombres `level`/`code`/`msg`/`targets` (NO `severity`/`message`); `targets` **siempre** array (nunca null).
  - Helper `chk(level, code, msg, targets)` equivalente al constructor del prototipo.
- **Fuera de alcance**: la lógica que produce los checks (E1-H05/H06).
- **Criterios de aceptación**:
  - `[Check{Err}, Check{Pass}].iter().map(|c| c.level).max() == Err` (test del bug de `§10` fila 4).
  - `serde_json::to_string(&Severity::Warn) == "\"warn\""`.
  - `serde_json` de un `CheckCode::OkfFm01` == `"OKF-FM01"`.
  - No existe ninguna otra definición de estos tres tipos en el workspace (grep en CI).
- **Dependencias**: E1-H01.
- **Pruebas**: round-trip serde de los 15 códigos; test del `.max()`.

### E1-H04 — Modelo de fichero: `Frontmatter` · `ParsedFile` · `FileKind` · `FmError`
- **Objetivo**: los tipos del fichero parseado, con los 7 `KNOWN_FM` tipados + `extra` y tags/timestamp RAW.
- **Referencias**: `ARCHITECTURE.md §4.1` · prototipo `parseFile`, `splitFront`, `parseYAML`.
- **Alcance**:
  - `Frontmatter` con `type`/`title`/`description`/`resource`/`tags`/`timestamp`/`status` + `#[serde(flatten)] extra: BTreeMap`.
    `status` es la 7ª KNOWN_FM **tipada** (dirige el ciclo `draft|review|accepted|deprecated`).
    `tags`/`timestamp` se guardan **RAW** (`serde_yaml::Value`) para poder detectar `FMT-TAGS`/`FMT-TS`.
  - `ParsedFile { kind: FileKind, fm: Option<Frontmatter>, fm_err: Option<FmError>, body: String, raw: String }`.
  - `FileKind { Concept, Index, Log }` (reserved = `kind != Concept`); `FmError { Missing, Unclosed, Malformed(String) }`.
- **Criterios de aceptación**:
  - `parse_file` NUNCA devuelve `Err` por contenido (FM01/02/03 son datos, no `Result`).
  - El orden de claves de `Frontmatter` respeta el que necesita `build_raw` (status incluido).
- **Dependencias**: E1-H01.
- **Pruebas**: parsear fixtures con FM ausente/sin cerrar/malformado → `fm_err` correcto.

### E1-H05 — Primitivas de modelo: parseo y serialización 1:1 del prototipo
- **Objetivo**: portar las funciones libres de parse/dump/build que sostienen todo lo demás.
- **Referencias**: `ARCHITECTURE.md §4` · prototipo `splitFront`, `parseYAML`/`miniYAML`,
  `dumpYAML`, `parseFile`, `buildRaw`, `normalize`, `resolveLink`, `outLinks`, `rawRelLinks`, `isISO`, `toISOStr`.
- **Alcance**:
  - `split_front`: separa frontmatter del cuerpo (reproduce el manejo de delimitadores `---` del proto,
    incl. los estados que generan `FmError::Unclosed`).
  - `parse_yaml`/`dump_yaml`: usar `serde_yaml` pero **conservando** la semántica del `miniYAML`/`dumpYAML`
    del proto donde difiera (orden de claves canónico para `build_raw`).
  - `build_raw`: reconstrucción canónica del `.md` (frontmatter ordenado status-aware + cuerpo). **Es la
    canonicalización** que el `§12` (CRDT) dice que sesga LWW por fichero — mantener determinista.
  - `resolve_link`: resolución de un href a `RelPath` (port fiel, incluye relativos vs absolutos `/…`).
  - `normalize`, `out_links`/`raw_rel_links`: extracción de enlaces salientes del cuerpo.
  - `is_iso`/`to_iso_str`: validación/normalización de timestamps ISO (alimenta `FMT-TS`).
- **Criterios de aceptación**:
  - `build_raw(parse_file(x)) == x` para todo `.md` ya canónico de las fixtures (idempotencia).
  - El arnés diferencial (E1-H18) confirma paridad byte a byte con el JS para estas funciones.
- **Dependencias**: E1-H04.
- **Pruebas**: idempotencia de `build_raw`; diferencial vs JS de cada función.

### E1-H06 — Conformidad: `validate_file` y los 15 checks OKF (+ `OKF-CONFLICT`)
- **Objetivo**: portar `validateFile`/`chk` → los 15 `CheckCode`, con `OKF-CONFLICT` nuevo (hard-fail).
- **Referencias**: `ARCHITECTURE.md §4.1`, `§10` fila 17, `§13.6.2` · prototipo `validateFile`
  (líneas ~1208–1245), `isRootish`, `isISO`.
- **Alcance**:
  - Reproducir cada check con su `level` exacto del proto:
    - `OKF-FM01/02/03` (err) — frontmatter ausente/sin cerrar/malformado (early-return como el proto).
    - `OKF-TYPE` (err si falta `type`; **pass** con mensaje si está) — **única regla dura**.
    - `REC-TITLE`/`REC-DESC` (info), `FMT-TAGS` (warn, tags no-lista), `FMT-TS` (warn, ts no-ISO).
    - `LINK-STUB` (info, enlaces a páginas inexistentes), `LINK-REL` (info, enlaces relativos),
      `ORPHAN` (info, nadie enlaza aquí), `BODY-STRUCT` (info, sin encabezados `^#{1,6}\s`).
    - `OKF-IDX` (warn, index con FM indebido), `OKF-LOG` (warn, fechas de historial no `AAAA-MM-DD`).
    - **`OKF-CONFLICT`** (err/hard-fail): marcadores `<<<<<<<`/`=======`/`>>>>>>>`/`|||||||` en cuerpo
      **o** frontmatter. NUEVO respecto al proto.
  - Mensajes **keyed por código** (i18n del `§12`): el texto humano se externaliza; el core produce
    `code` + `targets` + parámetros, no prosa hardcodeada en idioma. (El catálogo español canónico
    reproduce los textos del proto.)
- **Fuera de alcance**: el pase global de bundle (orphans/dangling) — eso es `analyze` (E1-H07).
- **Criterios de aceptación**:
  - Cada uno de los 15 códigos se dispara con la fixture correspondiente (E0-H03) y NO en las demás.
  - `OKF-CONFLICT` detecta marcadores tanto en cuerpo como en frontmatter y es `Severity::Err`.
  - El gate del proto (reserved-file gating, early-returns de FM01/02/03) se preserva.
- **Dependencias**: E1-H03, E1-H05.
- **Pruebas**: diferencial vs JS de `validateFile`; un test por código.

### E1-H07 — `analyze` (port de `analyzeBundle`) → `Analysis`
- **Objetivo**: el agregado de bundle: adyacencia, backlinks, índice, dangling, huérfanos, per-file, conteos.
- **Referencias**: `ARCHITECTURE.md §4.1` (`Analysis`), `§10` filas 4/5 · prototipo `analyzeBundle`
  (líneas ~1176–1206), `outLinks`, `resolveLink`.
- **Alcance**:
  - `Analysis { concepts, out: BTreeMap<RelPath,Vec<RelPath>>, inn, in_index: BTreeSet, dangling,
    orphans, per_file: BTreeMap<RelPath,Vec<Check>>, hard_fail, warn_count, okf_version }`,
    todo con `#[serde(rename_all="camelCase")]`.
  - `out` = adyacencia de **strings** (paths), `inn` = backlinks (invertir `out`),
    `in_index` derivado de qué `index.md` listan el concept.
  - `dangling` = hrefs que no resuelven; `orphans` = concepts sin inbound ni in_index.
  - `hard_fail = #ficheros con algún Check Err` (conteo, no `.max()` — `§10` fila 4); `warn_count` análogo.
  - `okf_version` = del `index.md` raíz; `None` si falta.
  - `per_file` reúne los checks locales (E1-H06) + los sintetizados por el pase global (LINK-STUB/ORPHAN).
- **Criterios de aceptación**:
  - Sobre la fixture conforme: `hard_fail == 0`. Sobre una con 1 Err + N Pass: `hard_fail == 1`.
  - `inn` es exactamente la inversa de `out`; `orphans`/`dangling` casan con el proto.
  - Diferencial vs JS de `analyzeBundle` sobre todas las fixtures.
- **Dependencias**: E1-H06.
- **Pruebas**: diferencial vs JS; property test (`inn` == invertir `out`).

### E1-H08 — `Bundle::from_files` + cache de `analyze` (`OnceCell`)
- **Objetivo**: el agregado `Bundle` construido desde un `FileMap`, con `analyze()` cacheado.
- **Referencias**: `ARCHITECTURE.md §4.2`, `§4.1` (`FileMap = BTreeMap<RelPath,String>`).
- **Alcance**:
  - `pub type FileMap = BTreeMap<RelPath, String>`.
  - `Bundle::from_files(files: FileMap) -> Self`; `analyze(&self) -> &Analysis` cacheado con `OnceCell`
    (recomputar es idempotente; el cache es solo perf).
  - Estructura interna que parsea cada fichero una vez (`parse_file`) y guarda el `ParsedFile`.
- **Criterios de aceptación**:
  - `analyze()` llamado dos veces devuelve la misma referencia (no recomputa).
  - Construir el bundle no toca el FS (puro).
- **Dependencias**: E1-H07.
- **Pruebas**: test de cacheo (contador de cómputo); construcción desde fixture.

### E1-H09 — Lectura semántica: `list_concepts` · `backlinks`
- **Objetivo**: portar `fileRow`/`renderTree` y el panel de backlinks a `ConceptSummary`/`Backlinks`.
- **Referencias**: `ARCHITECTURE.md §4.1` (`ConceptSummary`, `LinkRef`, `Backlinks`), `§4.2`
  · prototipo `fileRow`, `renderTree`, `renderLinks`, `dispTitle`, `resolveLink`.
- **Alcance**:
  - `list_concepts(&self) -> Vec<ConceptSummary>`: filas del árbol con `title` ya resuelto (fm.title o del
    path), `type`/`status`, `orphan`, `invalid` (= algún Check level=Err). La jerarquía la deriva el front del `path`.
  - `backlinks(&self, &RelPath) -> Backlinks { inbound: Vec<LinkRef>, index_refs, out, dangling }`:
    `inbound` con el `href` crudo usado (rich link-metadata del `§4.1`), `index_refs` = index.md que lo listan,
    `out` = destinos resueltos, `dangling` = hrefs salientes que no resuelven.
- **Criterios de aceptación**:
  - `ConceptSummary.invalid` true sii el fichero tiene algún `Check` Err.
  - `Backlinks.inbound[i].href` es el href tal cual aparece en el `.md` origen.
  - Diferencial vs JS de `fileRow`/`renderLinks`.
- **Dependencias**: E1-H08.
- **Pruebas**: diferencial; fixture con backlinks + dangling + index refs.

### E1-H10 — Grafo: `graph_model` · `neighborhood`
- **Objetivo**: portar `buildGraphModel` y la vecindad dirigida a `GraphModel`/`Neighborhood`.
- **Referencias**: `ARCHITECTURE.md §4.1` (`GraphModel`, `GraphNode`, `Edge`, `Neighborhood`),
  `§4.2` (`Direction`) · prototipo `buildGraphModel`.
- **Alcance**:
  - `GraphModel { nodes: Vec<GraphNode>, edges: Vec<Edge> }` (campo `edges`, NO `links`).
    `GraphNode { id, ghost, type, status }`; `Edge { source, target, dangling }`.
  - `ghost` = nodo destino que no existe como fichero (página por escribir).
  - `neighborhood(&self, p, depth: u32, dir: Direction) -> Neighborhood { root, nodes, edges }`.
  - `Direction { Out, In, Both }` (Out=dependencias, In=blast-radius, Both=mapa local).
- **Criterios de aceptación**:
  - `graph_model` marca ghosts correctamente; `edges[i].dangling` true para hrefs no resueltos.
  - `neighborhood(p, 1, Out)` solo incluye vecinos salientes a profundidad 1.
  - Diferencial vs JS de `buildGraphModel`.
- **Dependencias**: E1-H08.
- **Pruebas**: diferencial; tests de `depth`/`dir` sobre fixture en estrella.

### E1-H11 — Query: `tokenize_query` + `match_token` (semántica de subcadena)
- **Objetivo**: un único tokenizer/matcher con TODOS los quirks del proto.
- **Referencias**: `ARCHITECTURE.md §4.3`, `§10` fila 11 · prototipo `tokenizeQuery`, `matchToken`,
  `isPredicate`, `fieldMatch`, `valueIncludes`, `fmGet`, `fmPresent`, `applyQuery`, `matchFileQuery`.
- **Alcance**:
  - `query(&self, dsl: &str) -> Vec<RelPath>` (filtro de paths; port fiel, no enriquece).
  - Soporta: `field:val` (subcadena), `field=val` (exacto), `-neg`, `has:`/`no:`,
    `is:orphan|invalid|reserved|linked|accepted|draft|review|deprecated` (los 4 últimos = predicados de `status`),
    `body:`, texto suelto, y el **flip de negación** `!val` (un `!` inicial invierte `-neg`, doble-negable).
  - **Quirk obligatorio**: gating de fichero reservado **ANTES** de negar.
  - Nombre de campo ASCII `[\w\-]+` (clave con acento → cae a texto suelto); valor case-insensitive.
  - `body:`/texto suelto son **subcadena** (NO token FTS) — paridad con el proto.
- **Fuera de alcance**: aceleración FTS5 (vive en `store`, como superset; nunca único pre-filtro).
- **Criterios de aceptación**:
  - Cada operador y predicado tiene un test que coincide con el proto.
  - `!val` invierte; `-!val` doble-niega; el gating de reservado se aplica antes de negar.
  - Diferencial vs JS de `tokenizeQuery`+`matchToken` sobre un corpus de queries.
- **Dependencias**: E1-H08.
- **Pruebas**: corpus de queries (≥30) diferencial JS-vs-Rust.

### E1-H12 — `validate_draft`: validar contenido sin guardar
- **Objetivo**: `validate_draft(&self, fm, body) -> Vec<Check>` para feedback en vivo del editor/MCP.
- **Referencias**: `ARCHITECTURE.md §4.2` · prototipo `validateFile` aplicado a un draft.
- **Alcance**: corre los checks locales (E1-H06) + los que dependen del bundle (LINK-STUB contra el corpus actual)
  sobre un `(Frontmatter, body)` no persistido, devolviendo `Vec<Check>`.
- **Criterios de aceptación**: un draft con `type` vacío produce `OKF-TYPE` Err; con enlace a página inexistente, `LINK-STUB`.
- **Dependencias**: E1-H06, E1-H08.
- **Pruebas**: tests de draft conforme/no-conforme.

### E1-H13 — Escrituras validadas: `create_concept` · `merge_frontmatter` · `WriteOutcome`
- **Objetivo**: las escrituras OKF puras que devuelven un `WriteOutcome` (raw + hash + checks + rejected).
- **Referencias**: `ARCHITECTURE.md §4.1` (`FrontmatterPatch`), `§4.2`, `§10` fila 13 · prototipo
  `createConcept`, `offerCreate`, `bodyTemplate`, merge de frontmatter.
- **Alcance**:
  - `create_concept(&self, p, ty, …) -> WriteOutcome`: construye el `.md` canónico (`build_raw`),
    valida, y **rechaza por defecto** si introduce un `Err` (regla dura: `type` no vacío).
    Rechazo = `WriteOutcome { written: false, rejected: Some(motivo), … }` (NO un `Err`).
    Flag `allow_nonconformant` para forzar.
  - `merge_frontmatter(&self, p, patch: FrontmatterPatch) -> WriteOutcome`: semántica merge-patch RFC 7386
    (`Some(v)` escribe/reemplaza, `None` **borra**, clave ausente = no toca). `FrontmatterPatch(BTreeMap<String, Option<Value>>)`.
  - `WriteOutcome { path, raw, hash: [u8;32] (blake3), written, rejected, checks, bundle_hard_fail }`.
  - El `hash` blake3 es el que la workspace usará para echo-suppression (`§6`).
- **Fuera de alcance**: escribir a disco (lo hace la workspace, E5; el core solo computa el `raw`+outcome).
- **Criterios de aceptación**:
  - `create_concept` con `type` vacío → `written:false, rejected:Some(...)`, NO `Err`.
  - `merge_frontmatter` con `clave→None` borra esa clave del frontmatter resultante.
  - `hash` es blake3 del `raw`.
- **Dependencias**: E1-H06, E1-H08.
- **Pruebas**: rechazo no-conforme; null-borra; hash estable.

### E1-H14 — Generadores puros: `gen_index` · `gen_tag_indexes` → `Mutation`
- **Objetivo**: portar `genIndex`/`generateTagIndex` como funciones puras que devuelven un plan.
- **Referencias**: `ARCHITECTURE.md §4.2`, `§10` fila 12, `§12` (i18n: cabeceras canónicas fijas)
  · prototipo `genIndex`, `generateTagIndex`, `slugifyTag`.
- **Alcance**:
  - `gen_index(&self, dir: &str) -> Mutation` y `gen_tag_indexes(&self) -> Mutation`
    (`Mutation { writes: BTreeMap<RelPath,String>, deletes: Vec<RelPath> }`).
  - `gen_tag_indexes` **purga tags obsoletos** (de ahí los `deletes`).
  - **Cabeceras canónicas fijas** como consts (los bytes generados son ficheros commiteados; cambiar locale
    los churnea — `§12`). `slugify_tag` port fiel.
- **Fuera de alcance**: aplicar la `Mutation` y computar `{written,removed,unchanged}` (workspace, E5).
- **Criterios de aceptación**:
  - Generar dos veces sobre el mismo bundle produce `Mutation` idénticas (determinista, bytes estables).
  - Un tag eliminado del corpus aparece en `deletes`.
  - Diferencial vs JS de `genIndex`/`generateTagIndex`.
- **Dependencias**: E1-H08.
- **Pruebas**: determinismo de bytes; purga de tags; diferencial.

### E1-H15 — `add_log_entry` (changelog OKF) puro
- **Objetivo**: portar `addLogEntry` como transformación pura sobre `log.md`.
- **Referencias**: `ARCHITECTURE.md §13.7` (dos historiales: `log.md` curado), `§4.1` (`OKF-LOG`)
  · prototipo `addLogEntry`, `appendVersionLog`.
- **Alcance**: función que anexa una entrada `AAAA-MM-DD` al `log.md` respetando `OKF-LOG`; devuelve el nuevo raw / `Mutation`.
- **Criterios de aceptación**: entrada con fecha mal formada sería rechazada por `OKF-LOG`; el formato casa con el proto.
- **Dependencias**: E1-H06.
- **Pruebas**: diferencial vs JS; validación de formato de fecha.

### E1-H16 — Export: `export_zip`
- **Objetivo**: portar `exportBundle` a `export_zip<W: Write + Seek>`.
- **Referencias**: `ARCHITECTURE.md §4.2`, `§12` (seguridad: zip-slip) · prototipo `exportBundle`.
- **Alcance**:
  - `export_zip<W: Write + Seek>(&self, w: W) -> Result<(), CoreError>` que empaqueta el `FileMap` del bundle.
  - **Sin zip-slip**: como las claves son `RelPath` validados, las entradas del zip son rutas seguras.
- **Criterios de aceptación**: el zip resultante descomprime al mismo árbol de ficheros; ninguna entrada con `..`/absoluta.
- **Dependencias**: E1-H08.
- **Pruebas**: round-trip zip→FileMap; test de que un path malicioso nunca llega al zip.

### E1-H17 — Diff semántico OKF: `OkfDiff` y familia (`diffSnap` y cía.)
- **Objetivo**: portar `diffSnap`/`fmDiff`/`lineDiff`/`collapseDiff` al diff puro `OkfDiff` con LCS robusto.
- **Referencias**: `ARCHITECTURE.md §4.4` (familia `OkfDiff`), `§13.3`, `§10` filas 20/21 · prototipo
  `diffSnap`, `fmDiff`, `lineDiff`, `collapseDiff`, `suggestMsg`, `fmObj`, `fmFmt`.
- **Alcance**:
  - Tipos `§4.4`: `OkfDiff { files, generated, stats, status_changes, suggested }`,
    `FileDiff { path, kind: ChangeKind(Add|Mod|Remove), fm: Vec<FieldChange>, body: Vec<BodyHunk>,
    links_added, links_removed }`, `FieldChange { key, from, to }` (**orden status-first**),
    `BodyHunk { Context|Add|Remove|Gap(u32) }`, `StatusChange`, `DiffStats`, `MessageHint`.
  - `MessageHint` (i18n vía catálogo en la fachada): `AddSingle{title}`, `StatusSingle{to,title}`,
    `Update{added,modified,removed}`.
  - **Segregación de generados** (index/log/tags no cuentan como edición manual) → campo `generated`.
  - **Rendimiento (obligatorio, `§13.3`/`§10` fila 21)**: LCS del cuerpo con **DP en dos filas +
    Hirschberg** (no la matriz O(n·m)), **guarda de tamaño** (fallback grueso por umbral), y **saltar
    blobs binarios/no-UTF8**. Un fichero de 10k líneas NO debe reservar ~400 MB.
  - La firma toma **dos `FileMap`** (árbol vs árbol, o HEAD vs working) y devuelve `OkfDiff` — es el core
    el que da el *significado*; vcs solo provee los file-maps.
- **Fuera de alcance**: el contador "sin commitear" por hash (eso es comparación de hash por path, NO `OkfDiff` — vive en store/workspace).
- **Criterios de aceptación**:
  - `FieldChange` lista `status` primero cuando cambia.
  - Diff de un fichero de 10k líneas modificadas no supera un techo de memoria razonable (test con límite).
  - Cambios en index/tags/log aparecen en `generated`, no en `files` como edición manual.
  - Diferencial vs JS de `diffSnap`/`fmDiff`/`lineDiff`/`collapseDiff`.
- **Dependencias**: E1-H05, E1-H08.
- **Pruebas**: diferencial; test de memoria (Hirschberg); test de segregación de generados.

### E1-H18 — Arnés diferencial JS-vs-Rust (la red de seguridad)
- **Objetivo**: ejecutar el JS del prototipo (en Node) y el core Rust sobre las MISMAS fixtures y assertar paridad.
- **Referencias**: `ARCHITECTURE.md §12` (testing/paridad: "test diferencial proto JS vs core"), `§14`
  · todas las funciones portadas en E1.
- **Alcance**:
  - Extraer las funciones puras del `prototype/index.html` a un módulo JS invocable desde Node (sin DOM):
    `splitFront`, `parseFile`, `buildRaw`, `resolveLink`, `analyzeBundle`, `validateFile`, `chk`,
    `tokenizeQuery`, `matchToken`, `genIndex`, `generateTagIndex`, `diffSnap`/`fmDiff`/`lineDiff`/`collapseDiff`.
  - Un runner (`xtask diff-harness`) que: para cada fixture, corre JS → JSON y Rust → JSON y compara
    estructuralmente (normalizando orden donde el contrato usa `BTreeMap`/`BTreeSet`).
  - Integrar en CI: el harness verde es **precondición** para cerrar cualquier historia de E1.
- **Criterios de aceptación**:
  - El harness cubre todas las funciones de la lista y todas las fixtures de E0-H03.
  - Una discrepancia JS-vs-Rust hace fallar el job.
- **Dependencias**: E1-H05..E1-H17, E0-H03, E0-H08.
- **Pruebas**: el propio harness en CI.

### E1-H19 — Tipos de versionado en `core::types` (`Sha`, `CommitRow`, `OkfDiff` ya hecho, `Branch`, etc.)
- **Objetivo**: congelar la familia de tipos git de `§4.4` en el core (sin git2, sin I/O).
- **Referencias**: `ARCHITECTURE.md §4.4`, `§10` fila 20.
- **Alcance**:
  - `Sha(String)` newtype validado (hex de git; `git2::Oid` NUNCA cruza la frontera de vcs).
  - `CommitRow { id, short, message, author: Author, time_unix: i64, parents: Vec<Sha>, conformance: Option<CommitConformance> }`.
  - `Author { name, email }`, `CommitConformance { hard_fail, warn_count, conform }` (cruda, sin strictness).
  - `RepoState { Clean, Merging, Rebasing, CherryPicking, Reverting }`,
    `Branch { name, is_head, upstream, ahead, behind }`,
    `SyncOutcome { kind: SyncKind(Push|Pull), ok, summary }`.
  - Todo `#[serde(rename_all="camelCase")]` donde el `§4.4` lo marca.
- **Fuera de alcance**: cualquier lógica git (vive en `vcs`, E4). Aquí solo los tipos.
- **Criterios de aceptación**:
  - `Sha::new` rechaza no-hex; round-trip serde de toda la familia.
  - Estos tipos están definidos **una sola vez** (grep en CI: no hay `CommitMeta`/`VcsCommit`/`OkfDiff` duplicado).
- **Dependencias**: E1-H01.
- **Pruebas**: round-trip serde; validación de `Sha`.

### E1-H20 — Feature `schemars` (outputSchema MCP) y feature `render` (HTML de preview)
- **Objetivo**: gatear `#[derive(JsonSchema)]` en los DTO públicos y `pulldown-cmark` para el preview.
- **Referencias**: `ARCHITECTURE.md §3`, `§10` fila 14 · prototipo `mdRender`/`miniMd`.
- **Alcance**:
  - Feature `schemars`: detrás de ella, los DTO públicos del contrato derivan `JsonSchema` (lo consume el MCP, E7).
  - Feature `render`: `pulldown-cmark` para producir HTML de preview (port de `mdRender`/`miniMd`); el
    **saneado DOMPurify** vive en el frontend (`§12`), no aquí, pero el HTML generado debe ser apto para sanear.
- **Criterios de aceptación**:
  - `cargo build -p lodestar-core --features schemars` deriva `JsonSchema` sin tocar la API por defecto.
  - `--features render` expone la función de render; sin la feature, el core no arrastra `pulldown-cmark`.
- **Dependencias**: E1-H03, E1-H04 (DTOs), E0-H02.
- **Pruebas**: build con cada feature; snapshot del JsonSchema de `Check`/`Analysis`.
