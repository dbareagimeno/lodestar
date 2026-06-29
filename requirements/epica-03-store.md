# E3 — `lodestar-store` (SQLite/FTS5 + watcher)

> **Fase**: `§14.3`. **Objetivo de la épica**: la cache **derivada y desechable** en
> `<bundle>/.lodestar/index.db` (WAL, gitignored, siempre reconstruible), con watcher incremental y
> FTS5. `rusqlite` vive **SOLO aquí**. El test de paridad SQL==core es obligatorio: cuando podrían
> discrepar, **gana el core**. Referencias: `ARCHITECTURE.md §5`, `§10` filas 1/8/10/11, `§11`.

**Invariante de la épica**: `store` es **dueño único del DDL**. Materializa lo barato de mantener;
**sintetiza on-demand** lo que invalidaría en cascada (backlinks, orphans, ghosts, LINK-STUB, ORPHAN,
neighborhood, blast-radius). El watcher de este crate será el **único escritor** una vez compuesto en la
workspace (E5); aquí se construye el motor de upsert/gate.

---

### E3-H01 — DDL y apertura de la cache (`.lodestar/index.db`, WAL, `user_version`)
- **Objetivo**: crear/abrir la base SQLite con el esquema dueño del store y migraciones por `user_version`.
- **Referencias**: `ARCHITECTURE.md §5`, `§10` fila 10, `§12` (versionado: `user_version` ≠ `okf_version`).
- **Alcance**:
  - Apertura en `<bundle>/.lodestar/index.db` con `PRAGMA journal_mode=WAL`, `foreign_keys=ON`.
  - DDL (dueño único): `files` (frontmatter promovido a columnas: `type`/`title`/`description`/`status`/
    `resource` + `frontmatter_json` para el resto + `body`, `hash` blake3, `mtime`, `size`, `kind`),
    `links` (`src`, `dst`, `href`, **`src_is_index`** flag → `in_index` se deriva de aquí; **una sola tabla**),
    `tags` (`path`, `tag`), `diagnostics` (solo checks **locales**: `path`, `code`, `level`, `msg`, `targets_json`).
  - **Nombres de columnas casan con los nombres del `Check`** (`§10` fila 10).
  - `user_version` para migraciones de esquema de cache; reconstrucción total si la versión no coincide.
- **Fuera de alcance**: FTS5 (E3-H02), sintetizadas (E3-H05).
- **Criterios de aceptación**:
  - Abrir crea el fichero y el esquema; reabrir es idempotente.
  - Cambiar `user_version` fuerza rebuild limpio.
  - Una columna de `diagnostics` por cada campo del `Check`.
- **Dependencias**: E1-H03, E1-H04, E0-H01.
- **Pruebas**: crear/abrir/reabrir; migración por bump de `user_version`.

### E3-H02 — FTS5 externo sobre `(title, description, body)`
- **Objetivo**: tabla FTS5 que acelere búsquedas, como **superset**, nunca único pre-filtro de subcadena.
- **Referencias**: `ARCHITECTURE.md §5`, `§4.3`, `§10` fila 11, `§12` (seguridad: escapar FTS5).
- **Alcance**:
  - FTS5 "external content" sobre `files(title, description, body)`, sincronizada por triggers o por el upsert.
  - **Escapado de expresiones FTS5** (`§12` seguridad): las queries del usuario nunca se interpolan crudas.
  - Documentar y testear que FTS5 es **acelerador**: el resultado final SIEMPRE se confirma con la
    semántica de **subcadena** del core (`match_token`), porque FTS tokeniza y perdería matches de subcadena.
- **Criterios de aceptación**:
  - Una query de subcadena que FTS no encontraría (match parcial dentro de un token) **sí** aparece en el resultado final (vía core).
  - Una expresión FTS maliciosa (`"`/`*`/operadores) no rompe ni inyecta.
- **Dependencias**: E3-H01.
- **Pruebas**: test del caso subcadena-que-FTS-pierde; fuzz de escapado FTS.

### E3-H03 — Cold rebuild: `WalkBuilder` → `core::parse_file` → upsert transaccional
- **Objetivo**: reconstruir toda la cache desde el disco en una transacción.
- **Referencias**: `ARCHITECTURE.md §5`, `§11` (cold open 10k < ~2s).
- **Alcance**:
  - `ignore::WalkBuilder` (respeta `.gitignore`, excluye `.lodestar/`/`.git/`) → `core::parse_file` →
    upsert de `files`/`links`/`tags`/`diagnostics` en **una** transacción.
  - Computa los diagnostics **locales** vía `core` (no reimplementa checks).
  - Mide y cumple el presupuesto: cold open 10k concepts < ~2s (gate de bench, `§11`/E8-H12).
- **Criterios de aceptación**:
  - Tras un rebuild, las tablas reflejan el `FileMap`; segundo rebuild es idempotente.
  - Bench de 10k concepts dentro del presupuesto.
- **Dependencias**: E3-H01, E3-H02, E1-H06, E0-H03 (fixture 10k).
- **Pruebas**: rebuild sobre fixtures; bench gate.

### E3-H04 — Watcher incremental: `notify-debouncer-full` + gate mtime/size + **hash blake3**
- **Objetivo**: el motor incremental que descarta no-ops/echoes y hace upsert/delete + recompute del vecindario.
- **Referencias**: `ARCHITECTURE.md §5`, `§9` (gate por hash), `§10` fila 8, `§6` (echo-suppression), `§11` (edit→UI <150ms).
- **Alcance**:
  - `notify-debouncer-full` (~250 ms) → gate por **mtime+size** y **hash blake3** del contenido
    (descarta no-ops y **los echoes de nuestras propias escrituras**) → upsert/delete + recompute del
    vecindario afectado.
  - `reconcile_all()` repara drift tras tormentas de eventos.
  - El blake3 de la cache es **la única autoridad** de echo-suppression (`§6`): el `hash` se expone en los DTO.
- **Criterios de aceptación**:
  - Escribir el mismo contenido (no-op) NO genera un `IndexEvent`.
  - Un cambio real se refleja en < ~150 ms (edit→UI, bench).
  - `reconcile_all()` tras borrar/añadir ficheros fuera de banda deja la cache consistente.
- **Dependencias**: E3-H03.
- **Pruebas**: no-op suprimido; cambio reflejado; reconcile tras tormenta.

### E3-H05 — Síntesis on-demand: backlinks · orphans · ghosts · LINK-STUB/ORPHAN · neighborhood · blast-radius
- **Objetivo**: derivar (no materializar) lo que invalidaría en cascada, vía SQL/vistas y CTE recursivo.
- **Referencias**: `ARCHITECTURE.md §5`, `§4.2` (neighborhood/Direction), `§10` fila 10.
- **Alcance**:
  - Backlinks: índice sobre `links.dst`. Orphans/ghosts: vistas. `LINK-STUB`/`ORPHAN`: **sintetizados** (no en `diagnostics`).
  - Neighborhood (no dirigido) y **blast-radius direccional**: **CTE recursivo sobre aristas inversas**
    (distinto del neighborhood — `In`=impacto).
  - Estas consultas deben devolver lo mismo que el `core` equivalente (lo verifica E3-H07).
- **Criterios de aceptación**:
  - `LINK-STUB`/`ORPHAN` NO están materializados en `diagnostics` (se sintetizan al leer).
  - El CTE de blast-radius (`Direction::In`) casa con `core::neighborhood(.., In)`.
- **Dependencias**: E3-H01.
- **Pruebas**: paridad puntual de cada síntesis vs core (preludio de E3-H07).

### E3-H06 — Bus de eventos `IndexEvent` (crossbeam, runtime-neutral)
- **Objetivo**: emitir cambios por un canal síncrono que cada fachada puentea a su mundo.
- **Referencias**: `ARCHITECTURE.md §5`, `§9`.
- **Alcance**:
  - `crossbeam` `IndexEvent` síncrono (qué paths cambiaron, qué se invalidó). El MCP lo puentea a tokio;
    Tauri a `app.emit`; la CLI lo ignora.
  - API `subscribe() -> crossbeam::Receiver<IndexEvent>` (la expondrá la workspace en E5; aquí el productor).
- **Criterios de aceptación**: un upsert emite un `IndexEvent` con los paths afectados; sin suscriptores no bloquea.
- **Dependencias**: E3-H04.
- **Pruebas**: suscriptor de test recibe el evento tras un cambio.

### E3-H07 — Test de paridad obligatorio: SQL == `core::analyze`
- **Objetivo**: probar que `hard_fail`/backlinks/orphans/dangling vía SQL == vía core sobre la misma fixture.
- **Referencias**: `ARCHITECTURE.md §5` (cierre), `§10` fila 1, `§12` (testing/paridad).
- **Alcance**:
  - Para cada fixture: construir el `Bundle` (core) y la cache (store) y assertar igualdad de
    `hard_fail`, `warn_count`, backlinks (`inn`), orphans, dangling, `in_index`.
  - **Property test**: incremental (E3-H04) == rebuild (E3-H03) tras una secuencia aleatoria de ediciones (`§12`).
  - Si difieren: es **bug de la cache** (gana el core). El test lo deja claro en el mensaje.
- **Criterios de aceptación**:
  - Paridad verde sobre todas las fixtures.
  - El property test incremental==rebuild pasa con ≥100 secuencias aleatorias.
- **Dependencias**: E3-H03, E3-H04, E3-H05, E1-H07.
- **Pruebas**: el propio test de paridad + property test en CI.

### E3-H08 — `ConceptStore` trait (servir el API del core desde proyecciones SQL)
- **Objetivo**: permitir que, a escala, el motor de grafo/conformidad del core opere sobre proyecciones SQL en vez de todo el corpus en RAM.
- **Referencias**: `ARCHITECTURE.md §3` (trait `ConceptStore`), `§10` fila 1, `§11`.
- **Alcance**:
  - Definir el trait `ConceptStore` en el **core** (lectura abstracta de concepts) y una impl en `store`
    que sirve desde SQL (no materializa todos los cuerpos en RAM).
  - El core sigue puro: el trait no arrastra rusqlite (la impl vive en store).
- **Fuera de alcance**: reescribir todo el core para usarlo (es una vía de escalado; v1 puede seguir con `FileMap` en RAM para corpus pequeños).
- **Criterios de aceptación**:
  - `list`/`query`/`analysis` pueden servirse desde la impl SQL del trait y dan el mismo resultado que desde `FileMap`.
  - El core no declara `rusqlite` (sigue puro).
- **Dependencias**: E3-H01, E1-H08.
- **Pruebas**: paridad entre `Bundle::from_files` y el `Bundle` servido por `ConceptStore` SQL.
