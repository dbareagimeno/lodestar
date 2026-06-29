# E6 — `src-tauri` + frontend Svelte 5

> **Fase**: `§14.6`. **Objetivo de la épica**: la fachada de escritorio (Tauri v2) sobre `Workspace`,
> y el **port verbatim en aspecto** de la UI del prototipo a Svelte 5, **invirtiendo la propiedad de los
> datos**: el `files{}`/`analyzeBundle()` del proto se van a Rust; la webview es vista fina sobre un
> `BundleSnapshot` empujado. Incluye el pill/overlay/modo "Cambios" de git y la **isla imperativa** del grafo.
> Referencias: `ARCHITECTURE.md §7.1`, `§8`, `§13.7`, `§11`, `§10` filas 6/7.

**Reglas duras de la épica**: 100% del acceso a disco/diálogo en Rust (la webview no recibe `fs`/`shell`/
`dialog`); el watcher es el **único emisor** de cambios; nombres de comando/evento **congelados** en
constantes compartidas; `ipc.ts` **generado** desde los tipos Rust; el grafo es una **isla imperativa**
(nunca `{#each}` reactivo).

---

### E6-H01 — Tabla de comandos Tauri (nombres congelados) sobre `Workspace`
- **Objetivo**: implementar los comandos del `§7.1` como shells de 5–15 líneas que delegan en `Workspace`.
- **Referencias**: `ARCHITECTURE.md §7.1` (tabla congelada), `§7` (cero lógica OKF en fachadas).
- **Alcance**:
  - Comandos: `open_bundle` · `pick_dir` · `get_snapshot` · `list_concepts` · `read_concept` ·
    `write_concept` (enum `Raw|Structured`) · `create_concept` · `delete_concept` · `merge_frontmatter` ·
    `validate_draft` · `conformance` · `query` · `backlinks` · `neighborhood` · `graph_model` ·
    `generate_index` · `generate_tags` · `add_log_entry` · `export` · `get_settings` · `set_setting`.
  - Comandos `async` que delegan el trabajo pesado a `spawn_blocking`; **los guards `RwLock`/`Mutex` nunca
    cruzan un `.await`**.
  - Error `{code, message}` con `code` **estable** (`NO_BUNDLE` → onboarding).
  - Nombres pinned en una **constante compartida** (Rust) — fuente del `ipc.ts` generado.
- **Criterios de aceptación**:
  - Cada comando es un shell fino sin lógica OKF (la lógica está en `Workspace`/core).
  - Ningún guard de lock cruza un `.await` (revisión + lint si es posible).
  - `open_bundle` sobre un dir sin bundle devuelve `code: NO_BUNDLE`.
- **Dependencias**: E5-H03, E5-H04, E0-H06.
- **Pruebas**: tests de comando (mock workspace); golden cross-fachada (E7) sobre los que aplican.

### E6-H02 — Evento `bundle:changed` (único snapshot) + campo `vcs` barato + `vcs:changed`
- **Objetivo**: el único evento de snapshot empujado y el resumen git barato.
- **Referencias**: `ARCHITECTURE.md §7.1`, `§9`, `§13.7`, `§10` fila 7.
- **Alcance**:
  - Un solo evento `bundle:changed` con `{ snapshot: BundleSnapshot, changed: string[] }` (debounced),
    emitido por el **watcher** (único emisor). Los comandos mutadores devuelven su resultado **optimista**;
    el evento refresca decoraciones globales.
  - `bundle:changed` crece un campo `vcs` barato: `{head, branch, ahead, behind, pendingCount, clean}`.
  - `vcs:changed` desde el ref-watch (pista); el `OkfDiff`/log pesados se piden al abrir.
- **Criterios de aceptación**:
  - Editar un fichero emite **un** `bundle:changed` con el snapshot y los paths cambiados.
  - Un commit refleja el nuevo `head`/`pendingCount` en el campo `vcs` (optimista + reconcile).
- **Dependencias**: E6-H01, E5-H03, E4-H08.
- **Pruebas**: smoke "abrir bundle, editar, assertar snapshot poblado" (`§10` fila 7).

### E6-H03 — `ipc.ts` generado + registro de constantes compartido (mata la deriva de nombres)
- **Objetivo**: el contrato IPC tipado: `.d.ts` generado desde Rust + wrapper `ipc.ts` que importa los tipos.
- **Referencias**: `ARCHITECTURE.md §8` (contrato IPC), `§7.1`, `§10` filas 6/7.
- **Alcance**:
  - El `.d.ts` generado (E0-H04) ahora cubre todos los tipos del contrato y nombres de comando/evento.
  - `ipc.ts`: funciones tipadas que invocan cada comando (`invoke`) y suscriben eventos, importando los tipos generados.
  - **Sin capa DTO paralela** (principio #4): los tipos TS son los generados, no redeclarados.
- **Criterios de aceptación**:
  - Renombrar un campo en Rust + regenerar rompe la compilación de `ipc.ts` si el consumidor usa el nombre viejo.
  - No hay interfaces TS escritas a mano que dupliquen los tipos del contrato (revisión + grep).
- **Dependencias**: E6-H01, E0-H04.
- **Pruebas**: `tsc --noEmit` tras un cambio de nombre; ausencia de duplicados.

### E6-H04 — Stores Svelte: snapshot empujado como única fuente + `derived`
- **Objetivo**: el modelo de datos del front: el snapshot es la única fuente; tree/pill/backlinks/graph/perFile son `derived`.
- **Referencias**: `ARCHITECTURE.md §8`, `§11` (virtualización).
- **Alcance**:
  - Stores clásicos (`svelte/store`) con shapes verificables contra los tipos Rust: el `BundleSnapshot`
    empujado es la única fuente; `tree rows`, `conformance pill`, `backlinks`, `graph`, `perFile` son `derived`.
  - Writables: `bundleRoot`, buffers de edición por path (`OpenDoc` con baseline/dirty/inflight-hash),
    `query`, y estado efímero de vista/layout/tema. Runes ($state/$derived/$effect) solo para estado local de componente.
- **Criterios de aceptación**:
  - Un `bundle:changed` actualiza el snapshot y re-deriva tree/pill/backlinks sin código imperativo.
  - Los shapes de los stores casan con los tipos generados (test de tipos).
- **Dependencias**: E6-H03.
- **Pruebas**: tests de store (`§12`); derive correcto tras push.

### E6-H05 — Port verbatim de la UI: layout, árbol, tabs, temas (aspecto idéntico)
- **Objetivo**: replicar el aspecto del prototipo (mismo `<style>`, variables CSS, atributos `data-*`).
- **Referencias**: `ARCHITECTURE.md §8` · prototipo `renderTree`/`fileRow`/`dirRow`, `renderTabs`,
  `applyTheme`/`toggleTheme`, `applyLayout`/`loadLayout`/`toggleRail`/`startRailDrag`.
- **Alcance**:
  - Layout de rails redimensionables, árbol de concepts (virtualizado, `§11`), tabs, conmutador de tema,
    atributos `data-theme/view/explorer/rail-*` idénticos al proto.
  - La jerarquía del árbol se deriva del `path` de cada `ConceptSummary` (la deriva el front, `§4.1`).
  - **Virtualización de filas** del árbol para 10k concepts (`§11`).
- **Criterios de aceptación**:
  - Comparación visual con el prototipo: mismas variables CSS y `data-*`.
  - El árbol con 10k filas hace scroll fluido (virtualizado).
- **Dependencias**: E6-H04, E0-H05.
- **Pruebas**: snapshot visual/CSS; perf de scroll con fixture 10k.

### E6-H06 — Editor multi-escritor: buffers, echo-suppression y banner de conflicto
- **Objetivo**: el editor que no pierde ediciones sin guardar y distingue su echo de una edición externa.
- **Referencias**: `ARCHITECTURE.md §8` (editor multi-escritor), `§6` (echo-suppression por hash).
- **Alcance**:
  - Los pushes del snapshot **nunca pisan** un buffer sin guardar (`OpenDoc` con baseline/dirty/inflight-hash).
  - Supresión de echo: usa el `hash` que devuelve cada escritura (distingue mi write volviendo por el watcher
    de una edición externa, que **sí** levanta un **banner de conflicto**).
  - Modos de editor del proto (`renderEditor`/`renderForm`/`renderRaw`/`renderPreview`/`renderReserved`/`updateModeControls`).
  - `write_concept` con enum `Raw|Structured`; `validate_draft` en vivo (feedback de conformidad sin guardar).
- **Criterios de aceptación**:
  - Una edición externa concurrente a un buffer dirty levanta el banner (no pisa el buffer).
  - Mi propio write volviendo por el watcher NO levanta banner (echo suprimido por hash).
- **Dependencias**: E6-H04, E6-H01, E1-H12.
- **Pruebas**: simulación de write externo vs echo propio; preservación de buffer dirty.

### E6-H07 — Preview de markdown con **DOMPurify** (no regex casera)
- **Objetivo**: render del cuerpo a HTML saneado.
- **Referencias**: `ARCHITECTURE.md §8`, `§12` (seguridad: DOMPurify) · prototipo `mdRender`/`miniMd`.
- **Alcance**:
  - Render del markdown (el HTML lo produce `core` con feature `render`, E1-H20, o un renderer JS equivalente)
    **saneado con DOMPurify** antes de inyectarlo. **Prohibida** la "regex casera" del proto para saneado.
- **Criterios de aceptación**:
  - Un `.md` con `<script>`/`onerror=`/`javascript:` se renderiza sin ejecutar nada (saneado).
  - El aspecto del preview casa con el del proto para markdown benigno.
- **Dependencias**: E6-H05, E1-H20.
- **Pruebas**: corpus XSS → saneado; paridad visual de markdown benigno.

### E6-H08 — Grafo: isla imperativa `createStarMap` (diff-merge, Barnes-Hut, sin `{#each}`)
- **Objetivo**: portar el grafo como isla imperativa que posee el SVG y el loop rAF, escalable a 10k nodos.
- **Referencias**: `ARCHITECTURE.md §8` (isla imperativa), `§11` (Barnes-Hut), prototipo `createStarMap`
  equivalente: `buildGraphModel`, `drawGraph`, `startSim`/`simStep`, `paintGraph`/`paintInto`, `startDrag`,
  `setGraphScope`, `openMap`/`closeMap`/`openGraphBig`, `GPOS` (mapa de posiciones), `starTint`/`updateGraphMatchInfo`.
- **Alcance**:
  - `createStarMap(svg)` posee el SVG, el loop rAF de física y el **mapa persistente de posiciones** (`GPOS`).
  - Svelte lo monta y le pasa **nodos/aristas/actual/matched por métodos dentro de `$effect`**, **nunca** con
    `{#each}` reactivo → cambios de topología hacen **diff-merge** (preservan layout); selección/búsqueda = repintados O(1).
  - **Escala (`§11`)**: **Barnes-Hut/quadtree** (la sim all-pairs O(n²) del proto no escala), cap del scope global
    (clustering o por defecto "vecindad"), 60 fps hasta N nodos visibles.
- **Criterios de aceptación**:
  - Un cambio de topología preserva las posiciones existentes (diff-merge, no recrea el SVG).
  - Selección/búsqueda repintan sin recomputar layout (O(1)).
  - La sim usa Barnes-Hut (no all-pairs); 60 fps con la fixture grande hasta N visibles.
- **Dependencias**: E6-H04, E1-H10.
- **Pruebas**: preservación de layout en cambio de topología; bench de fps; verificación de que no hay `{#each}` para nodos.

### E6-H09 — Store `vcs` + pill de git + popover
- **Objetivo**: el pill (HEAD/rama/ahead-behind/pendientes) y su popover, alimentados por el resumen barato.
- **Referencias**: `ARCHITECTURE.md §13.7` (frontend), `§13.1` (vocabulario directo), `§13.6.4` (pill nunca obsoleto).
- **Alcance**:
  - Store `vcs` alimentado de `vcs:changed` + el resumen barato del `bundle:changed`.
  - **Pill**: HEAD/rama/ahead-behind/pendientes con **vocabulario git directo** (commit/rama/push/pull, sin eufemismos).
  - **Popover**: pendientes, recientes, cambiar de rama, push/pull, "restaurar al último conforme".
  - Update **optimista** con el `Sha` del commit + reconcile al enfocar.
- **Criterios de aceptación**:
  - El pill muestra "N sin commitear"/"Limpio" y la rama actual; tras commit se actualiza optimista.
  - El popover ofrece push/pull (deshabilitados sin upstream/sin `git` en PATH, con aviso).
- **Dependencias**: E6-H02, E6-H04.
- **Pruebas**: estados del pill; popover con/sin upstream.

### E6-H10 — Overlay de historial (timeline + conformidad por commit + propuestas en revisión)
- **Objetivo**: el overlay con el timeline de la rama, puntos de conformidad, propuestas y panel de diff.
- **Referencias**: `ARCHITECTURE.md §13.7`, `§13.4` (conformidad perezosa), `§13.8` (propuestas = `status:review`)
  · prototipo `openVerOverlay`/`renderVerOverlay`/`renderVerTimeline`/`verRowEl`/`badgeFromConf`/`renderVerDiff`.
- **Alcance**:
  - Timeline de la rama con **puntos de conformidad por commit renderizados progresivamente** (off-thread,
    punto "computando…", persistidos — `§13.4`).
  - "Propuestas en revisión" = concepts con `status: review` (**NO** ramas, `§13.8`); aceptar/rechazar = editar frontmatter.
  - Panel de diff (`OkfDiff`) + restaurar/comparar/filtrar, todo en una página.
  - `vcs_log`/`vcs_diff`/`vcs_last_conforming`/`vcs_restore` desde la fachada.
- **Criterios de aceptación**:
  - Los puntos de conformidad aparecen progresivamente sin bloquear la UI.
  - "Propuestas en revisión" lista `status:review` y NO crea ramas.
  - "Restaurar al último conforme" usa `last_conforming` y pasa por checkpoint (E5-H05).
- **Dependencias**: E6-H09, E4-H09, E5-H05.
- **Pruebas**: render progresivo; propuestas = status:review; restore con checkpoint.

### E6-H11 — 4º modo de editor "Cambios" (diff de la página vs HEAD, `OkfDiff` perezoso)
- **Objetivo**: el modo que muestra el diff semántico de la página abierta vs HEAD.
- **Referencias**: `ARCHITECTURE.md §13.7`, `§13.3` (OkfDiff perezoso, solo el fichero abierto), `§10` fila 21
  · prototipo `renderDiffMode`/`renderDiffBody`/`diffCardEl`/`fieldRow`/`lifecycleEl`.
- **Alcance**:
  - 4º modo "Cambios" junto a editor/raw/preview/reserved: `OkfDiff` de la página vs HEAD, **computado perezoso**
    solo para el fichero abierto (`vcs_diff_working`).
  - Renderiza frontmatter por-campo (status-first), cuerpo con hunks/gaps, transiciones de ciclo de vida.
  - El grafo/física **no se toca**.
- **Criterios de aceptación**:
  - El modo "Cambios" muestra el `OkfDiff` solo del fichero abierto (no diffea todo el bundle).
  - Sin cambios → estado vacío; con cambios → hunks correctos (paridad con `diffSnap`).
- **Dependencias**: E6-H06, E1-H17, E5-H05.
- **Pruebas**: diff perezoso del fichero abierto; paridad de render con el proto.

### E6-H12 — Diálogo de commit + opt-in de `log.md` + estados de conflicto/firma
- **Objetivo**: el diálogo que dispara `commit` con mensaje sugerido y opt-in de changelog.
- **Referencias**: `ARCHITECTURE.md §13.7` (dos historiales), `§13.5`, `§13.6.3`, `§13.8` (firma)
  · prototipo `openCommitDlg`/`commitVersion`/`suggestMsg`.
- **Alcance**:
  - Diálogo de commit (Ctrl/Cmd+S) con **mensaje sugerido** (`MessageHint` del core, localizado en la fachada).
  - **Opt-in** de anexar al `log.md` curado (viaja en el **mismo** commit; no se auto-sincroniza con `git log`).
  - Estados: `RepoState::Merging` → bloquea con "resolviendo conflicto"; repo que exige firma → avisa y ofrece commit vía CLI.
- **Criterios de aceptación**:
  - El mensaje sugerido casa con `MessageHint` (Add/Status/Update).
  - Con merge en curso, el diálogo bloquea el commit; con firma exigida, avisa.
  - El opt-in de `log.md` produce una sola entrada en el mismo commit.
- **Dependencias**: E6-H09, E5-H05, E1-H15.
- **Pruebas**: mensaje sugerido por escenario; bloqueo en merge; opt-in log.md.

### E6-H13 — Onboarding / first-run en GUI + "activar git"
- **Objetivo**: el flujo de crear/abrir bundle y activar git desde la GUI.
- **Referencias**: `ARCHITECTURE.md §12` (first-run), `§13.2` (estados sin `.git`), `§7.1` (`NO_BUNDLE`).
- **Alcance**:
  - `code: NO_BUNDLE` → pantalla de onboarding (crear/abrir bundle, `pick_dir`).
  - "Crear bundle" = `vcs_init` + scaffold (`§12`); "activar git" cuando `discover` da `None`.
  - En cada `open` de repo existente, verificar que `.lodestar/` está ignorado (oferta "dejar de trackear").
- **Criterios de aceptación**:
  - Abrir un dir sin bundle muestra onboarding; "crear" deja un bundle con git inicializado.
  - Un repo con `.lodestar/` trackeada ofrece dejar de trackearla.
- **Dependencias**: E6-H01, E5-H01, E4-H02.
- **Pruebas**: onboarding NO_BUNDLE; activar git; oferta de untrack.

### E6-H14 — e2e smoke de Tauri
- **Objetivo**: una prueba e2e que abre la app, abre un bundle, edita y verifica el snapshot/decoraciones.
- **Referencias**: `ARCHITECTURE.md §12` (testing: e2e smoke de Tauri), `§10` fila 7.
- **Alcance**: test e2e (WebDriver/Playwright contra el binario Tauri) del flujo abrir→editar→assertar pill/tree/conformidad.
- **Criterios de aceptación**: el e2e pasa en CI (al menos una plataforma); cubre el smoke del `§10` fila 7.
- **Dependencias**: E6-H01..E6-H06.
- **Pruebas**: el propio e2e en CI.
