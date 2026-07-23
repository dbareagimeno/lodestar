# Estado de implementación

> Mapea las épicas/historias de [`requirements/`](requirements/) a su estado real en esta rama.
> Construido en el **orden de fases ratificado** (`ARCHITECTURE.md §14`), validando con tests en cada fase.
>
> **Resumen**: **todas las épicas (E0–E8) están implementadas y verificadas.** Backend completo
> (core + store SQLite/FTS5+watcher con paridad SQL==core + vcs con switch/merge/hooks + workspace con
> bus en vivo + CLI + MCP con golden cross-fachada) y **escritorio completo** (fachada Tauri v2 con la
> tabla de comandos congelados + evento `bundle:changed`, y UI Svelte 5 funcional: árbol, editor
> multi-escritor, isla del grafo, modo Cambios). **~113 tests** en verde; `cargo clippy --workspace
> --all-targets --all-features --locked -- -D warnings` limpio; `cargo doc -D warnings` limpio;
> `npm run check`/`build` del frontend en verde. Ya hay **pipeline de release multiplataforma**
> (`release.yml`: macOS arm64 · Windows · Linux, bundles **sin firmar**) y **CI multiplataforma**.
> Lo pendiente es **producto/pulido**, no arquitectura (firma/notarización de bundles, rails
> redimensionables, rmcp, `.d.ts` generado): ver [`DECISIONES.md`](DECISIONES.md).

## Cómo correrlo

```bash
cargo test --workspace          # ~113 tests (incl. 6 diferenciales JS-vs-Rust; core, store, cli, vcs, workspace, mcp)
cargo run -p lodestar-cli -- check --path <bundle>     # la puerta de CI (exit 0/1)
cargo run -p lodestar-cli -- log | last-conforming | branch | switch | merge | hooks
cargo run -p lodestar-mcp [-- --root <dir>]            # servidor MCP por stdio (raíz = cwd)
```
(La app de escritorio —`frontend/` + `src-tauri/`, binario `lodestar-desktop`— se retiró de `main`
a la rama `experimental/ui-desktop`; ya no se construye ni se ejecuta desde este repo headless.)

## Estado por épica

| Épica | Estado | Detalle |
|---|---|---|
| **E0** Scaffolding | ✅ Hecho | Cargo workspace con 7 crates + direcciones del §3; `#![forbid(unsafe_code)]` en core; fixtures; CI (fmt/clippy/test + frontend); frontend Svelte/Vite. |
| **E1** `lodestar-core` | ✅ Hecho | Contrato de tipos congelado, modelo, conformidad (15 checks + OKF-CONFLICT), analyze, query, grafo, generadores, export, diff. **Arnés diferencial JS-vs-Rust (H18, §12)**: 6 fixtures corren las funciones puras del prototipo (vía node) y comparan con el core — la red de paridad. La auditoría halló y corrigió **6 divergencias** (NFC en slugs, orden numérico de tags con `sort_paths_cmp`, `null` en `yaml_is_empty`/`fm_present`, aristas a reservados en el grafo, orden de aparición de extras vía `IndexMap`). 22 + 6 tests. |
| **E2** `lodestar-cli` | ✅ Hecho | `check` (humano/--json/--sarif), `index`/`tags` (--check→exit 4), `export`, `init`; exit codes congelados. 8 tests. |
| **E3** `lodestar-store` | ✅ Hecho | DDL dueño único (`files`/`links`/`tags`/`diagnostics` + FTS5 + `commit_conformance`), cold rebuild, watcher `notify-debouncer-full` con **gate por hash blake3**, síntesis SQL (backlinks/orphans/dangling/blast-radius CTE), FTS5 con escapado, bus `IndexEvent` (crossbeam), trait `ConceptStore`. **13 tests**: paridad SQL==core, property incremental==core (120 ediciones), watcher en vivo, FTS. |
| **E4** `lodestar-vcs` | ✅ Hecho | libgit2 local + red por binario `git` + **resolve_rev**, **staged_files**, **switch** (sin tocar working tree), **merge** (3-vías a nivel de árbol con marcadores + `MERGE_HEAD`), **install_hooks**, **tree_oid**. Cache de conformidad por tree-oid en el store, cableada en la workspace. **12 tests**. |
| **E5** `lodestar-workspace` | ✅ Hecho | Handle unificado, único escritor, snapshot, commit/restore con checkpoint, switch/merge, conformidad cacheada por tree-oid, config (`lodestar.toml`), y **bus de eventos en vivo** (`open_live`/`enable_cache`/`subscribe`) con **update optimista** de la cache tras cada escritura. **12 tests**. |
| **E6** Tauri + frontend | ✅ Hecho | **Fachada Tauri v2** real: comandos congelados sobre `Workspace` + estado del bundle + forwarder del bus `IndexEvent` → evento `bundle:changed` (UI en vivo). Binario `lodestar-desktop` compila; CI de Rust instala webkit y construye el frontend antes. **Frontend Svelte 5 funcional**: layout de 3 columnas colapsables, árbol filtrable, editor multi-escritor con validación y diagnósticos localizados, panel de enlaces, **isla imperativa del grafo** (`createStarMap`, SVG+rAF, sin `{#each}`), modo **Cambios** (diff + commit). `npm run check`/`build` verdes. Pulido en [`DECISIONES.md §2`](DECISIONES.md). |
| **E7** `lodestar-mcp` | 🟢 Parcial | 13 tools sobre la workspace + bucle JSON-RPC por stdio (stdout puro). **Golden cross-fachada** (tool==workspace) + e2e. **5 tests**. Pendiente: transporte `rmcp` oficial + resources (ver [`DECISIONES.md §3`](DECISIONES.md)). |
| **E8** Transversales | 🟢 Parcial | Hechos: exit codes/SARIF, escritura atómica, **zip-slip cerrado por RelPath en `import`**, identidad de commits + override por `lodestar.toml`, trailer Co-Authored-By del agente, gitignore de `.lodestar/`, **config por-bundle (`lodestar.toml`: strictness + identidad)**, **`lodestar import`** (zip del prototipo o dir), **`init` con git init + commit inicial real**, **i18n keyed por código** (catálogo español), **arnés diferencial JS-vs-Rust (§12)**, y **pipeline de release multiplataforma** (`release.yml`: macOS arm64/Windows/Linux → bundles sin firmar + binarios CLI/MCP; Release en borrador) con **CI multiplataforma** (job de Rust en las 3 plataformas). Pendiente: **firma/notarización** de bundles + updater, gate de bench (§11), threat model. |

## Infraestructura de proceso (2026-07-10)

El repo tiene ahora una **estructura de agentes y skills** para el desarrollo por venir
(SDD · TDD · BDD · jueces ciegos · contratos de frontera) — mapa y workflows en
[`.claude/README.md`](.claude/README.md):

- **Agentes** (`.claude/agents/`): `planificador` (spec/diseño mayor → épica de historias) ·
  `historiador` · `autor-tests` · `implementador` · `juez-historia` (ciego: solo spec+diff) ·
  `guardian-contrato`.
- **Skills** (`.claude/skills/`): `/planificar` (features grandes: diseño + épica, 2 puertas) ·
  `/historia` · `/tdd` · `/juzgar [--panel]` · `/contrato [--check]` · `/mutantes` · `/ciclo`
  (pipeline completo por historia).
- **Contratos de la frontera** (`contracts/`): `ipc.yml` (comandos Tauri + eventos) y `mcp.yml`
  (13 tools), extraídos del código real; los tipos siguen viviendo solo en `core::types`
  (invariante #4). Verificación con `/contrato --check`.
- **Mutation testing a demanda**: `cargo-mutants` configurado (`.cargo/mutants.toml`), sin CI.
- Primera historia acordada para el nuevo flujo: **ts-rs** (E0-H04/E6-H03,
  [`DECISIONES.md §4`](DECISIONES.md)).

## Cobertura de historias (destacadas)

- **E1**: H01–H19 implementadas y testeadas, **incluida H18** (arnés diferencial JS-vs-Rust:
  `prototype/harness/` corre las funciones puras del prototipo en node y `tests/differential.rs` compara con
  el core sobre 6 fixtures — analyze · query · generadores · grafo, con el prototipo como oráculo). H20
  (schemars/render) como features.
- **E2**: H01–H05 hechas; H06/H07 (reindex/import/git) reales o stub según fase.
- **E4**: H01–H06, H09 (conformidad por commit) hechas; H07 (red) hecha; H08/H10 parciales.
- **E5**: H01–H06 hechas (sin el watcher de E3); H07 parcial.
- **E7**: H01–H06 hechas (subset stdio; H06 = golden cross-fachada en `tools.rs` + e2e); H07 doc.

## Revisión profunda (2026-07): endurecimiento transversal

Auditoría completa por subsistema (core/store/vcs/workspace/cli/mcp/tauri/frontend) con
verificación empírica; ~40 defectos corregidos con tests de regresión. Lo más relevante:

- **Paridad core↔prototipo** (8 arreglos): escalares no-string en el frontmatter ya no invierten
  el veredicto (`type: 123` era OKF-FM03 hard-fail; el proto coerce con `String()`), `null`
  explícito cuenta como presente y `buildRaw` lo conserva, `isISO` valida el string entero,
  `titleFromPath` con `\b\w` de JS, tags ordenados con `localeCompare`, `fmDiff` sin cambios
  fantasma y con orden de aparición, `diffSnap` con `sortPaths`, panel de backlinks con
  dedupe/sin-self/sin-reservados.
- **Seguridad**: `RelPath` rechaza también unidades Windows (`C:\…` → zip-slip en `import`) y
  backslashes; validación de raíz de bundle en MCP y Tauri (`open_bundle` ya no indexa un
  directorio arbitrario).
- **Ciclo de merge completo** (vcs/workspace): el commit que concluye un merge lleva **2 padres**
  y limpia `MERGE_HEAD` (antes el repo quedaba `Merging` para siempre); ff con HEAD desacoplado
  es error, no éxito silencioso; el index de git se sincroniza tras switch/ff (fin de la suciedad
  fantasma y los checkpoints vacíos); los conflictos en artefactos **generados** (index/tags) se
  auto-resuelven regenerando.
- **Robustez de la cache**: un `.md` no-UTF8 ya no congela todas las reconciliaciones;
  `busy_timeout` para multi-proceso; el watcher ignora `.lodestar/`/`.git/` (fin del eco por
  cada escritura de la propia cache); `index.db` corrupto se recrea solo; watcher arranca antes
  del rebuild (sin ventana ciega); `search` de la cache usa la misma función del core.
- **Escritura atómica de verdad**: `fsync` antes del rename + temporal único por proceso.
- **CLI**: `--staged/--rev/--range` excluyentes (exit 2), `import` sin arg = uso (2, no 3),
  `init` sin arg usa el CWD (no el bundle ancestro), `lodestar.toml` inválido = exit 3 (no
  defaults silenciosos), push/pull fallido = 3 (no el 1 de conformidad), hook pre-commit usa
  `check --staged` (§13.5).
- **MCP**: `inputSchema` en las 13 tools, `-32700`/`-32600`/`ping`, errores de tool como
  `isError` (visibles para el modelo), `structuredContent` siempre objeto.
- **Tauri**: forwarder con `Weak<Store>` (fin de la fuga de hilo+conexión y de los snapshots
  del bundle anterior al reabrir), coalescing de ráfagas de eventos, comandos `async` (no
  bloquean el hilo de UI), recuperación de Mutex envenenado, `vcs:changed` emitido tras commit.
- **Frontend**: selección del grafo reparada (el repintado síncrono mataba el `click`),
  editor sin carrera de cargas ni pisado de escrituras externas (y con confirmación al
  descartar), `type`/`status` del árbol salen del grafo del core, espejo `types.ts` completado.
- **Tests e2e nuevos**: viaje completo CLI (init→check→generadores→export/import), hooks con
  `git` real, push/pull contra remoto local bare, ramas switch/merge, protocolo MCP por stdio.

## First-run del escritorio (2026-07)

- **El IPC de la webview estaba muerto**: `frontend/src/lib/ipc` usa `window.__TAURI__` pero
  `tauri.conf.json` no activaba `withGlobalTauri` → NINGÚN comando funcionaba desde la UI
  (abrir por ruta incluido). Activado.
- **Crear/abrir workspace con selector nativo de carpetas** (`tauri-plugin-dialog`): comandos
  `pick_folder` y `create_bundle`; el scaffold vive UNA vez en `Workspace::init_bundle`
  (compartido con `lodestar init`).
- **Opener rediseñado**: tarjeta de bienvenida con «Crear workspace nuevo» / «Abrir existente»
  (diálogo nativo), ruta manual como alternativa, estados de carga/error, y oferta de crear el
  workspace ahí mismo cuando la ruta indicada no es un bundle. El topbar oculta tabs/píldora
  hasta que hay workspace abierto.

## Release y CI multiplataforma (2026-07)

- **Pipeline de release** (`.github/workflows/release.yml`): se dispara con el tag `vX.Y.Z`, compila
  **macOS Apple Silicon (arm64)**, **Windows** y **Linux**, y crea un GitHub Release en **borrador**
  con los bundles (dmg/deb/appimage/nsis) + los binarios de CLI/MCP. Bundles **sin firmar** (firma/
  notarización diferida — ver [`DECISIONES.md`](DECISIONES.md)). `bundle.active = true` y los iconos
  de marca (estrella dorada) integrados. Runbook en [`RELEASING.md`](RELEASING.md).
- **CI multiplataforma**: el job `rust` (fmt/clippy/build/test/doc) corre en `ubuntu-latest`,
  `macos-14` y `windows-latest` (`fail-fast: false`); el paso de `apt` (webkitgtk/soup) queda
  condicionado a Linux. `core-purity` y `frontend` siguen solo en Linux. Coste ~3× minutos, asumido
  por ser producto multiplataforma.
- **Sincronización de versión**: `scripts/set-version.sh X.Y.Z` fija la versión en `Cargo.toml`
  (`[workspace.package]`), `src-tauri/tauri.conf.json` y `frontend/package.json` con `sed` acotado.
- **crates.io**: preparado (orden topológico + `publish = false` en fixtures/tauri) pero **sin
  publicar** (repo privado; publicar es público y permanente). Ver [`RELEASING.md`](RELEASING.md).

## Invariantes verificados

- **Core puro**: `lodestar-core` no declara `tauri`/`rusqlite`/`notify`/`tokio`/`git2`; `#![forbid(unsafe_code)]`.
- **Una sola verdad computada**: la conformidad por commit (vcs) y el gate (cli) usan el **mismo** `core::analyze`.
- **Un solo contrato de tipos**: definido una vez en `core::types`; el front lo refleja (a generar con ts-rs).
- **RelPath**: newtype validado; único chokepoint de path-traversal (tests de absolutas/`..`).
- **git vocabulario directo + transporte híbrido**: libgit2 local, binario `git` solo para red.
- **Único escritor**: la workspace escribe `.md` atómico (temp+rename); nadie más escribe.

## Próximos pasos (todo opcional — producto/pulido, ver [`DECISIONES.md`](DECISIONES.md))

Las 9 épicas (E0–E8) están implementadas. Lo que queda no es arquitectura:

1. **Empaquetado** (§1): plataformas objetivo, iconos de marca y pipeline de release **ya hechos**
   (`release.yml`, tres plataformas, bundles sin firmar); queda la **firma/notarización** + **updater**.
2. **Pulido de UI** (§2): rails redimensionables por arrastre, overlay de grafo, resaltado con la
   semántica del core.
3. **E0-H04/E6-H03** (§4): generar el `.d.ts` desde Rust (ts-rs/specta).
4. **E7** (§3): adoptar `rmcp` oficial + resources cuando un cliente lo exija.
5. **E8** (§9): gate de bench (§11), threat model.

## Giro a motor headless de integridad semántica (E9–E14) — COMPLETO

Refactor de `docs/REFACTOR.md`, diseño ratificado en `ARCHITECTURE.md §19` (supersede §13 en
superficie de producto; git queda como crate dormido) y `DECISIONES.md §0`. Descomposición en
`requirements/epica-09..14` (47 historias, orden E9→E14).

- **E9 — Reducción de alcance** (Fase 0):
  - ✅ **E9-H01** — Retirar las tools git del MCP (`history`/`last_conforming_commit`/`commit`);
    MCP pasa de 13 a 10 tools. `contracts/mcp.yml` recortado. Juez ciego: APROBADA (3/3).
  - ✅ **E9-H02** — Retirar los subcomandos git de la CLI (`log`/`last-conforming`/`branch`/
    `switch`/`merge`/`pull`/`push`/`hooks`) y los flags `--staged`/`--rev`/`--range` de `check`
    (D-check). `check` sin flags juzga el working tree; `reindex` conservado (movido a
    `commands.rs`). `git.rs` eliminado. Juez ciego: APROBADA (4/4).
  - ✅ **E9-H05** — Config `.lodestar/config.yaml` (YAML): tipo `WorkspaceConfig` +
    `WorkspaceConfig::load` (writableRoots/referenceRoots/ignored + gate + transactions; identity
    dormida). Defaults seguros; `RelPath` rechaza traversal en roots; malformado = error explícito.
    Convive con el `Config`/`lodestar.toml` legado. Juez ciego: APROBADA CON RESERVAS (4/4).
    (Reserva del merge de `ignored` cerrada en E9-H06.)
  - ✅ **E9-H06** — Separación canónico vs runtime: `.gitignore` gestionado como texto plano desde
    workspace (sin git2) ignorando solo `.lodestar/index.db` + `.lodestar/runtime/` (idempotente,
    con adopción de repos de estilo viejo); scaffold de `.lodestar/runtime/{plans,receipts,staging}`;
    `WorkspaceConfig::load` inyecta siempre los `ignored` obligatorios (cierra la reserva de H05).
    Juez ciego: APROBADA (4/4).
  - ✅ **E9-H03** — Aislado `lodestar-vcs` como crate dormido: `cargo tree -p lodestar-mcp`/
    `-p lodestar-cli` confirman que `vcs`/`git2` solo llegan **transitivamente** vía
    `lodestar-workspace` (ningún `use lodestar_vcs`/`vcs` en `crates/lodestar-{mcp,cli}/src/`);
    doc-comment de módulo en `crates/lodestar-vcs/src/lib.rs` declarando el crate DORMIDO
    (puntero a `ARCHITECTURE.md §19`/`§13`). `cargo test -p lodestar-vcs` sigue verde (12 tests);
    `cargo build --workspace` sin warnings nuevos. No se tocó el crate ni `core::types`.
  - ✅ **E9-H04** — UI congelada en el flujo de desarrollo: `.claude/README.md` y
    `docs/WORKFLOWS.md` anotan que el motor es headless y que `/ciclo`/`/historia`/`/ux` no tocan
    `frontend/`/`src-tauri/` en v2; el skill `/ux` y el agente `disenador-ux` quedan marcados
    **no aplicables al giro headless** (documentados, no invocados — reconciliado con el circuito
    UX preexistente sin revertirlo). `CLAUDE.md` actualizado (estado + mapa de crates con
    `lodestar-app`) sin reescribir los invariantes #1–#6.
    - **Superado por `remove-ui-from-main`**: la UI de escritorio se retiró después de `main` a la
      rama `experimental/ui-desktop` (con `frontend/`, `src-tauri/`, `contracts/ipc.yml`, el espejo
      `types.ts`, el skill `/ux` y el agente `disenador-ux`). El flujo ya no la trata como
      «congelada» sino como **retirada**; docs y `.claude/` reencuadrados en consecuencia.
  - ✅ **E9-H07** — Documentación de producto reposicionada: `README.md`/`CLAUDE.md` describen el
    posicionamiento como motor headless de integridad semántica, citan `ARCHITECTURE.md §19`,
    listan `lodestar-app` en el mapa de crates y marcan git como capacidad dormida y la UI como
    congelada. Este bloque de `IMPLEMENTATION_STATUS.md` refleja E9 completa.
- **E9 — COMPLETA** (H01–H07, las 7 historias de la fase 0).
- **E10 — COMPLETA (13/13)** (esquemas + lectura headless):
  - ✅ **E10-H03** — `ConceptRevision` + `WorkspaceRevision` en `core::types` (puros): revisión
    determinista sobre `writableRoots` (excluye `.lodestar/`, referenceRoots, mtime/orden/caché;
    contención por segmentos; separador `\0` anti-colisión). Juez ciego: APROBADA CON RESERVAS (4/4).
  - ✅ **E10-H06** — Extensión de `Check` (campos opcionales `id`/`range`/`related`/`fixes`,
    retro-compat: `fixes`/`related`→`[]`, `id`/`range` ausentes) + familias estáticas de `CheckCode`
    (`SCHEMA-REQFIELD`/`SCHEMA-STATUS`/`REL-TARGET`/`REL-CARD`/`REL-TYPE`). Frontend congelado sin
    tocar. Juez ciego: APROBADA CON RESERVAS (2/2). **Pendiente en E10-H07**: emitir `Check.msg`
    español por cada código nuevo (equivale a la "i18n" en headless).
  - ✅ **E10-H01** — Crate nuevo `lodestar-app` (fino sobre `Workspace`, D1-C): `Envelope<T>`
    (7 claves wire camelCase, D3), `ResourceLink`, `App::open`. Deps directas sin rusqlite/git2/tokio.
    Juez ciego: APROBADA (2/2).
  - ✅ **E10-H02** — `ErrorCode` (16 códigos SCREAMING_SNAKE) en `core::types` + mapeo
    `CoreError`/`WorkspaceError`→`ErrorCode` y `ErrorEnvelope` (code/message/recovery) en
    `lodestar-app`. Juez ciego: APROBADA CON RESERVAS (3/3). **A rastrear en E12/E13**: hacer que
    `WorkspaceError::Core` preserve la variante `CoreError` (hoy la aplana a String → un
    `PERMISSION_DENIED` real se degradaría a `INTERNAL_IO_ERROR` al envolverse).
  - ✅ **E10-H04** — `ConceptRef {path, id?}` + `ConceptId` en `core::types`; `App::resolve_ref`
    resuelve contra `Analysis::concepts` (invariante #3: excluye reservados) →
    `CONCEPT_NOT_FOUND` si no existe; `AMBIGUOUS_REFERENCE` reservado. Juez ciego: APROBADA (3/3).
  - ✅ **E10-H05** — `core::schema` (PURO): `Schema`/`DocType`/`RelationDef`/`FieldDef` (wire
    camelCase) + loader `WorkspaceSchema::load` en workspace (ausente→`Schema` permisivo,
    malformado→Err). Juez ciego: APROBADA (3/3).
  - ✅ **E10-H07** — `validate_schema(bundle, schema) -> Vec<Check>` puro y aditivo (SCHEMA-REQFIELD
    por campo obligatorio ausente, SCHEMA-STATUS por status fuera de allowedStatuses; msg español —
    cierra la reserva de H06). No se llama desde `analyze` (diferenciales intactos); se compondrá en
    E10-H12 (knowledge_check). Juez ciego: APROBADA CON RESERVAS (3/3).
  - ✅ **E10-H08** — Tool `workspace_status` (1ª tool headless): `App::workspace_status(profile)`
    con la forma §9.1 (workspaceRevision, counts desde Analysis, capabilities por perfil,
    recovery). Server MCP acepta `--profile readonly|standard`; shell fino que delega en el servicio.
    Juez ciego: APROBADA (2/2). (Drift de mcp.yml diferido a E10-H13.)
  - ✅ **E10-H09** — Tool `knowledge_search` (sustituye `query`): casado por `Bundle::query`
    (subcadena del core, invariante #3) ∩ `Analysis::concepts`; filtros types/statuses/tags/pathPrefix;
    snippet UTF-8-safe, `revision`, SIN `body` (estructural); orden determinista (score desc, path asc)
    + paginación por cursor-offset autosuficiente. Juez ciego: APROBADA CON RESERVAS (3/3).
    **A vigilar**: filtros avanzados (is:orphan/references/…) se admiten pero se ignoran en silencio
    (implementarlos en E11/E10-H13); cursor malformado reinicia a página 1.
  - ✅ **E10-H10** — Tool `knowledge_get`: `include` selectivo (campo no pedido no se puebla),
    `revision` siempre, backlinks/diagnostics/outgoing desde la verdad del core (invariante #3),
    selección de secciones por `headingPath` (rangos por nivel de heading, excluye hermanas), error
    en forma wire (`CONCEPT_NOT_FOUND`). Juez ciego: APROBADA CON RESERVAS (3/3). **A arreglar en
    E12-H04 (edit_section)**: `parse_headings` no reconoce code fences (un `#` dentro de ``` se toma
    como heading → puede truncar el rango).
  - ✅ **E10-H11** — Tool `schema_inspect`: modos `catalog`/`type` proyectan el `Schema` cargado
    (`WorkspaceSchema::load`); `DocType` reexpuesto de core::schema sin DTO paralelo (invariante #4);
    sin schema → catálogo vacío; modo/tipo inválido → `INVALID_SCHEMA` en wire. Juez ciego: APROBADA (3/3).
  - ✅ **E10-H12** — Tool `knowledge_check` (sustituye `conformance_check`): compone `analyze`
    (OKF) + `validate_schema` (E10-H07, cableado por 1ª vez) con scopes workspace/concept/paths/
    affected (vecindario vía `neighborhood`, sin off-by-one); ids de diagnóstico estables
    (`diag:blake3:` solo de datos del diagnóstico); `conformant`/`summary` computados antes de
    minimumSeverity/paginación. Juez ciego: APROBADA (3/3).
  - ✅ **E10-H13** — `outputSchema` (schemars) en las 5 tools nuevas, derivado del tipo Rust real
    (`schema_for!`, no divergible); `contracts/mcp.yml` reescrito (15 tools: 10 heredadas + 5 nuevas)
    + sección de migración §15; core sigue puro con la feature schemars. Retirada de query/
    conformance_check **descopada** a la limpieza final de superficie al cerrar E13. Juez ciego:
    APROBADA CON RESERVAS (2/2).
  - **E10 — COMPLETA** (13/13). Criterio de salida cumplido: un agente puede comprender y auditar
    la base (workspace_status/knowledge_search/knowledge_get/schema_inspect/knowledge_check) sin
    tocar el filesystem. **Pendiente al cierre de E13**: limpieza final de mcp.yml → 10 tools objetivo
    (retirar query/conformance_check/find_*/neighborhood/create/update/generate según reemplazos).
- **E11 — COMPLETA (5/5)** (grafo e impacto):
  - ✅ **E11-H01** — Tool `graph_query` (consolida backlinks/outgoing/neighborhood/orphans/dangling):
    reexpone `Bundle::neighborhood`/`backlinks` y `Analysis::orphans`/`dangling` (invariante #3, paridad
    literal); truncación + cursor; outputSchema; `mcp.yml` actualizado (las 4 tools viejas se retiran
    en la limpieza final de E13). Juez ciego: APROBADA CON RESERVAS (4/4). (Reserva de `node_for`
    resuelta en E11-H02.)
  - ✅ **E11-H02** — `path_between` (BFS), `cycles` (Tarjan SCC iterativo), `components` (BFS no
    dirigido) puras en `core::graph` (reusan `graph_model`, invariante #3; deterministas) + enchufadas
    en `graph_query`. Reserva de H01 resuelta: `node_for` público, `graph_node_for` eliminado.
    Diferenciales 6/6 verde. Juez ciego: APROBADA (4/4).
  - ✅ **E11-H03** — `validate_relations(bundle, schema)` puro (REL-TARGET si el target no existe,
    REL-TYPE si su type no está en target_types, REL-CARD si cardinality "one" con >1 target; msg
    español + range al campo), cableado aditivo en `knowledge_check`. Diferenciales verde. Juez
    ciego: APROBADA CON RESERVAS (3/3).
  - ✅ **E11-H04** — Validación de paths externos (`referenceRoots`): `Workspace::external_refs`
    (`implemented_by`/`verified_by` → `{path,exists}` + diagnóstico `EXTREF-MISSING`) y
    `assert_writable` (referenceRoots → `PERMISSION_DENIED`, contención por segmentos);
    `knowledge_get.externalReferences` cableado. **Seguridad**: un juez ciego cazó un oráculo de
    existencia por `join` crudo (traversal/absolutas); endurecido con `RelPath::new`+`under_root`
    antes de tocar disco + test de regresión `ref_externa_traversal`. Re-juicio: APROBADA CON
    RESERVAS (drift menor del espejo types.ts, sin impacto en la webview). Nuevo `CheckCode::ExtrefMissing`
    y `WorkspaceError::PermissionDenied`.
  - ✅ **E11-H05** — Tool `impact_analyze`: directlyAffected (backlinks directos), transitivelyAffected
    (neighborhood(In) del core; paridad con store::blast_radius verificada), blockingReferences (relaciones
    tipadas entrantes del schema, para delete; decoy de enlace suelto excluido), risk (high con bloqueos),
    recommendations. Juez ciego: APROBADA (3/3). Minor: `relation_field_targets` duplica
    `core::schema::relation_targets` (privada) — promover a público en una limpieza futura.
  - **E11 — COMPLETA** (5/5). Criterio de salida cumplido: Lodestar responde preguntas estructurales
    (graph_query: backlinks/outgoing/neighborhood/orphans/dangling/path_between/cycles/components) y
    anticipa consecuencias (impact_analyze), con relaciones tipadas (REL-*) y paths externos validados.
- **E12 — COMPLETA (9/9)** (planificación de cambios):
  - ✅ **E12-H01** — Tipos del plan en `core::types`: `ChangeSetId`/`PlanHash`/`ReceiptId`, `ChangeSet`
    (wire `baseWorkspaceRevision`/`planHash`/`expiresAt`), `NormalizedOperation` (11 variantes),
    `RiskAssessment`/`RiskLevel` (low/medium/high), `SemanticDiff`, `ValidationReport`. `FrontmatterPatch`
    ganó serde. Juez ciego: APROBADA (2/2).
  - ✅ **E12-H02** — `core::plan::assess_risk` (pura): mide el blast-radius de deprecate/delete/move
    (`Bundle::backlinks`); umbral 0→sin factor, 1..=4→Medium, >=5→High; level=máximo, reasons español.
    Juez ciego: APROBADA (2/2).
  - ✅ **E12-H03** — `core::plan::semantic_diff(before, after, schema)` (pura): created/modified/
    deleted/*_changes reusan `diff_snap`; diagnosticsIntroduced/Resolved = diff de all_checks
    (analyze+validate_schema+validate_relations) por clave (targets,code,msg). `moved` vacío (diff_snap
    no detecta renames → H06/H08). Juez ciego: APROBADA (3/3).
  - ✅ **E12-H04** — `core::plan::validate_result(bundle, schema)` → `ValidationReport` (reusa
    all_checks; conformant=errors==0 explícito) + `PlanPolicy{requireConformantResult,allowWarnings}`
    + `can_apply(report, policy)` (los dos ejes). Juez ciego: APROBADA (2/2).
  - ✅ **E12-H05** — Normalización de contenido: `normalize_create` (usa bodyTemplate + {title}),
    `normalize_replace_text` (error si conteo != expectedOccurrences), `normalize_edit_section` (acota
    por headingPath). Lógica de secciones MOVIDA a `core::model` (pública) con **fix de code fences**
    (cierra la reserva de E10-H10); `knowledge_get` la reusa (sin duplicar). Juez ciego: APROBADA CON
    RESERVAS (3/3). **A cubrir en E12-H08**: normalizadores `patch_frontmatter`/`replace_body` (los 11
    ops) + modos Append/Prepend de edit_section.
  - ✅ **E12-H06** — Normalización de estructura: `normalize_move` (1 Move + N ReplaceBody reescribiendo
    los entrantes; discrimina el enlace por `resolve_link`, no regex; preserva estilo/fragmentos) y
    `normalize_delete` (reject→`CoreError::InboundLinksExist`, remove_links→Delete + desenlazar entrantes).
    Juez ciego: APROBADA CON RESERVAS (3/3). **A endurecer antes de E13**: `Retarget`/`CreateStub` hoy hacen
    solo Delete en silencio (deben implementarse o dar error explícito); añadir test de enlace-señuelo y
    cobertura de rutas relativas.
  - ✅ **E12-H07** — Normalización semántica: `normalize_add_relation`/`remove_relation` (validan
    RelationDef → `RELATION_CONSTRAINT_VIOLATION`), `normalize_transition_status` (valida allowedStatuses),
    `normalize_apply_fix`. `validate_relations` emite un `Fix{safe}` en REL-TARGET (fix_id blake3 estable,
    aditivo sin regresión); apply_fix lo re-localiza y materializa (quita la relación rota). Juez ciego:
    APROBADA (3/3).
  - ✅ **E12-H08** — Tool `change_plan` (integración central, perfil standard): dispatcher de los 11
    ops crudos → normalizadores del core; `apply_normalized_ops` construye el bundle hipotético EN
    MEMORIA (no escribe, invariante #1); semantic_diff + assess_risk + validate_result + impact;
    planHash determinista (blake3 de baseWorkspaceRevision + normalizedOperations, SIN reloj);
    REVISION_CONFLICT por-op (ConceptRevision) y a nivel workspace. Cierra reserva de H05
    (patch_frontmatter/replace_body). outputSchema + mcp.yml. Juez ciego: APROBADA (4/4).
    **A rastrear**: gating por perfil (readonly debe rechazar tools de cambio) → E14-H03.
  - ✅ **E12-H09** — Persistencia del plan: `change_plan` escribe el `PlanResult` a
    `.lodestar/runtime/plans/<hex>.json` (nombre saneado sin `:`, runtime desechable); `App::load_plan`
    con caducidad (`expiresAt` pasado → `PLAN_EXPIRED`; reloj solo en app). El plan no afecta
    `WorkspaceRevision` (runtime excluido, invariante #1). Juez ciego: APROBADA (3/3).
  - **E12 — COMPLETA** (9/9). Criterio de salida cumplido: un agente puede proponer refactors complejos
    sin modificar archivos (change_plan normaliza/simula/valida en memoria, con diff semántico, riesgo,
    validación, concurrencia optimista y plan persistido/recuperable).
- **E13 — COMPLETA (11/11)** (publicación recuperable):
  - ✅ **E13-H01** — Staging: `Workspace::materialize_staging(&ChangeSet)` computa el resultado con
    `apply_normalized_ops` y lo escribe en `.lodestar/runtime/staging/<id saneado>/` SIN tocar el
    canónico (invariante #1; runtime desechable); `validate_staging` construye el Bundle del resultado,
    aplica el gate estricto y limpia + `NONCONFORMANT_RESULT` si no conforme. `WorkspaceError::
    NonconformantResult`. Juez ciego: APROBADA (2/2).
  - ✅ **E13-H02** — Lock de workspace: `acquire_lock` con creación atómica exclusiva
    (`create_new` = O_CREAT|O_EXCL, sin TOCTOU) en `.lodestar/runtime/lock.json`; `WorkspaceLock` RAII
    cuyo Drop libera best-effort (seguro en unwind, sin doble-panic). `reverify_base_revision` →
    `WRITE_CONFLICT` si la revisión cambió. Juez ciego: APROBADA (3/3). (Lock huérfano ante SIGKILL → H06.)
  - ✅ **E13-H03** — Write-ahead journal: `create_journal` escribe `.lodestar/runtime/journal/<txnId>.json`
    en estado `prepared` (ops `pending`) con fsync ANTES de la 1ª sustitución; `mark_applied` marca la op,
    transiciona `prepared`→`applying` y re-persiste con fsync. JSON de recuperación estable (camelCase +
    estados lowercase) que H06 releerá. Juez ciego: APROBADA CON RESERVAS (2/2). **A endurecer en H05/H06**:
    reescritura temp+rename+fsync-del-dir (hoy truncate+write → posible JSON torn ante crash) y recovery
    tolerante a journal torn.
  - ✅ **E13-H04** — Copias de recuperación: `Workspace::backup_originals(txn, affected)` copia
    byte-a-byte (fs::copy) cada original existente a `.lodestar/runtime/recovery/<txn>/` y marca los
    ausentes ("no existía") en un manifiesto `.absent`; solo LEE el canónico (invariante #1). `RecoveryDir`
    con path/backup_path/was_absent. Juez ciego: APROBADA CON RESERVAS (2/2). (Assert del manifiesto `.absent`
    → H06.)
  - ✅ **E13-H05** — Aplicación atómica por lote: `Workspace::publish(change_set, journal)` aplica los
    cambios al canónico SOLO por el único escritor (`io::write_atomic` temp+fsync+rename para creados/
    modificados, `io::delete` para borrados; orden determinista por `RelPath`; invariante #5), marca el
    journal por op y lo sella (`applied`), y devuelve la `WorkspaceRevision` resultante (== la prevista).
    Endureció `write_journal` a escritura atómica (temp+rename+fsync-dir), cerrando la reserva de H03.
    Juez ciego: APROBADA (3/3). **A resolver en H06/H08**: el journal debe crearse con el conjunto
    completo de paths afectados que `publish` calcula (no solo las ops crudas), p. ej. para `Move`.
  - ✅ **E13-H06 ⭐** — Crash-recovery determinista: `Workspace::recover()` escanea los journals no-`done`
    y decide por el ESTADO DURABLE — `applied`→completar (canónico ya es el resultado, limpia), `prepared`/
    `applying`→restaurar desde las copias de H04 (deshace renames parciales por `write_atomic`, borra los
    creados vía `.absent`). Sin ventana de corrupción (`mark_all_applied` sella tras el último rename; la
    restauración deriva el conjunto del árbol de recovery). NUNCA un `.md` parcial (property `recovery_sin_
    parciales` sobre 7 FailPoints × 2 formas). Gate `guard_recovery` bloquea escrituras con
    `WORKSPACE_RECOVERY_REQUIRED` (publish excluye su propio journal). Tolerante a JSON torn. Juez ciego
    riguroso: APROBADA (4/4). **Contratos a honrar en E13-H08**: change_apply debe llamar `recover()`, hacer
    `backup_originals` ANTES de `publish`, y crear el journal con el conjunto afectado completo.
  - ✅ **E13-H07** — `ChangeReceipt` (creado en core::types, forma REFACTOR §6.5) + retención:
    `write_receipt` persiste `.lodestar/runtime/receipts/<id>.json` (temp+fsync+rename); `gc_receipts`
    ordena por mtime y purga los excedentes (>maximumReceipts) y caducados (retainReceiptsFor) más
    antiguos, borrando también su `recovery/<id>/`. Vínculo receipt↔recovery por id saneado (contrato para
    H08). Juez ciego: APROBADA CON RESERVAS (2/2).
  - ✅ **E13-H08** — Tool `change_apply` (perfil standard): `App::change_apply` orquesta los 15 pasos —
    load_plan (PLAN_EXPIRED) → verificar planHash recomputado (PLAN_STALE, sin escribir) →
    `Workspace::apply_transaction` [lock → recover si pendiente → afectados reales → assert_writable
    (PERMISSION_DENIED antes de tocar el canónico) → staging+validar → reverify → **backup → journal →
    publish** → sellar] → receipt + gc. Honra los contratos de H06 (backup y journal antes de publish,
    conjunto afectado completo, recover). PERMISSION_DENIED no se degrada (assert_writable directo).
    Juez ciego riguroso: APROBADA (4/4). **Diferido**: gating por perfil (E14-H03); fsync del árbol de
    recovery para power-loss (hardening E14).
  - ✅ **E13-H09** — Tool `change_revert` (perfil standard): `App::change_revert` verifica el receipt
    (ausente/purgado → `PLAN_EXPIRED`), la revisión actual == `result_revision` (si no → `WRITE_CONFLICT`,
    sin tocar disco) y las copias de recuperación; luego `Workspace::revert_transaction` restaura desde
    `recovery/<orig>/` como una transacción INVERSA recuperable (lock + backup del estado actual + journal
    propios ANTES de restaurar por `write_atomic`/`delete`). El workspace vuelve a `previousRevision`.
    Juez ciego: APROBADA (3/3).
  - ✅ **E13-H10** — Auditoría `.lodestar/runtime/audit.jsonl`: `change_apply`/`change_revert` son
    wrappers que auditan SIEMPRE antes de devolver (éxito → result:"success" + revisiones; fallo,
    incluido un RevisionConflict que aborta antes de publicar → result = código wire). Best-effort
    (no tumba la operación ni enmascara el error); append JSONL; runtime (invariante #1); SystemTime
    solo en app (invariante #2). Juez ciego: APROBADA CON RESERVAS (2/2).
  - ✅ **E13-H11** — Auto-regeneración de `index`/`tags` dentro de `change_apply` (decisión D6a): la
    transacción de publicación fusiona EN MEMORIA (`transaction.rs::augment_with_regenerated`) lo que
    producirían `lodestar index` (regenera los `index.md` de directorio ya existentes, excluyendo
    `tags/`) y `lodestar tags` (`gen_tag_indexes`: escribe los vigentes y PURGA los obsoletos) sobre el
    resultado del plan → `result_augmented`. El conjunto afectado se deriva contra el resultado
    aumentado, de modo que staging+validar → **backup → journal → publish** cubren index/tags en el
    MISMO lote/journal/receipt (único escritor, recuperable igual que un `Move`). Idempotente
    (afectados por-diferencia). `materialize_staging`/`publish` conservan firma (núcleo extraído a
    `*_result`). Sin tools MCP de generación (D6a). Tests `apply_regenera_index`/`apply_regenera_tags`
    en `crates/lodestar-app/tests/regen.rs`. Juez ciego riguroso: APROBADA CON RESERVAS (2/2).
    **Reservas menores registradas** (no bloqueantes): (1) `gen_tag_indexes` SIEMPRE materializa el
    árbol `tags/` vigente (fiel a `lodestar tags`), asimétrico con index que solo regenera existentes
    → en un bundle con tags pero árbol `tags/` sin generar, cualquier apply lo materializa; (2) sin
    test de crash dedicado que mate la publicación DESPUÉS del `.md` del plan y ANTES del index/tags
    regenerado (la recuperabilidad de ese path queda garantizada estructuralmente: está en
    affected/journal/backup).
- **E14 — COMPLETA (6/6)** (integración software + evaluación — `ARCHITECTURE.md §19.8`):
  - ✅ **E14-H01** — `lodestar check` como puerta de CI con conformidad schema-driven: `check` (working
    tree, sin flags git) juzga con el MISMO motor que `App::knowledge_check` scope `workspace` (OKF +
    SCHEMA-* + REL-* + refs externas). La fusión OKF+schema/rel vive en UN solo sitio compartido
    (`App::schema_diagnostics_by_path`), consumido por `knowledge_check` y por `App::full_analysis`
    (invariante #3, una sola verdad computada; sin doble `analyze()`). La CLI es fachada fina que
    consume `full_analysis` y deriva `conformant` con la misma regla del motor. Salida humana / `--json`
    (campo `conformant` aditivo + `perFile` con los `SCHEMA-*`/`REL-*`) / SARIF (`ruleId` schema/rel)
    surfacean los diagnósticos del motor completo, no solo el veredicto. Exit codes CONGELADOS
    (`0`/`1`/`2`/`3`) intactos; `blocked` es superconjunto del anterior (nada que bloqueaba deja de
    hacerlo). Sin cambios en `core::types` (invariante #4; `conformant` inyectado en la fachada). Tests
    `check_falla_schema`/`check_conforme_json`/`check_caza_edicion_directa` + surfacing
    `check_sarif_lista_schema`/`check_json_lista_schema` en `crates/lodestar-cli/tests/cli.rs`. Juez
    ciego (2 pasadas): la 1ª APROBADA CON RESERVAS (salida no surfaceaba schema/rel) → cerrada con
    micro-ciclo rojo→verde; la 2ª (historia completa, no-regresión MCP 41 tests verdes) **APROBADA
    (6/6)**. Hallazgos menores heredados (no bloqueantes): (1) `check` abre el `App` completo → puede
    materializar la cache `store` como efecto de un comando read-only (mismo camino que MCP, coherente
    con invariante #5); (2) `conformant` juzga solo `concepts` mientras `gate_blocked` cuenta `Err` de
    todos los ficheros (p. ej. `index.md`) → un error solo en `index.md` da `conformant:true` pero exit
    1 vía gate — es exactamente la semántica de `knowledge_check` que la historia manda replicar.
  - ✅ **E14-H02** — Convivencia con proyectos de software (config por proyecto + detección de escritura
    externa): **historia de composición/regresión** — el comportamiento ya emerge de E9-H05
    (`writableRoots`/`referenceRoots`/`ignored` en `WorkspaceConfig`) + E11-H04 (`assert_writable` →
    `PERMISSION_DENIED`, paso 5 de `apply_transaction`, ANTES de tocar disco) + E13-H02/H08
    (`reverify_base_revision` → `WRITE_CONFLICT`, paso 7, ANTES de publicar). Aporta la **cobertura de
    integración e2e que faltaba** (ninguna prueba ejercitaba el orquestador `apply_transaction`
    completo): `crates/lodestar-workspace/tests/convivencia.rs` con `solo_escribe_writable` (create bajo
    `src/` → rechazo sin tocar disco; create bajo `knowledge/` → se aplica) y `detecta_escritura_externa`
    (edición externa entre plan y apply cambia la revisión writable → `WRITE_CONFLICT`, el `.md` conserva
    la edición externa). **CERO cambios de producción.** Juez ciego: **APROBADA (2/2)**, no-vacuidad
    verificada (las aserciones dependen realmente del enforcement/reverify). Ítems del **alcance** sin
    criterio testeable propio (no exigidos como aceptación, anotados): `ignored`
    (`node_modules`/`target`/`.git`, ya cubierto por `ignored_conserva_obligatorios` en `workspace.rs`) y
    "al reabrir/tras evento recalcular/invalidar revisiones y reindexar" (`REFACTOR §5.3`, ejercitado
    indirectamente vía reverify que relee la revisión del disco).
  - ✅ **E14-H03** — Instrucciones del servidor + perfiles para agentes (FRONTERA mcp.yml): perfiles
    `readonly`/`standard` (`--profile`, default standard; el enum `Profile` ya venía de E10-H08). **Fuente
    única** de "tools de cambio": `tools::CHANGE_TOOLS = [change_plan, change_apply, change_revert]` +
    `is_change_tool`, de la que derivan TANTO el filtrado de `tools/list` (`available_tools(profile)`)
    COMO el gating de invocación (`available(profile, name)`), gobernados por `Profile::writes_enabled()`
    — sin lista duplicada que pueda divergir. Bajo `readonly`: las 3 tools de cambio se ocultan de
    `tools/list` Y su invocación se rechaza con `-32602` ANTES del despacho (`main.rs`, antes de
    `tools::call()`) — **cierra la reserva de gating por perfil de E13-H08**: ocultar de la lista no
    basta, un cliente que las llame igualmente no planifica/aplica/revierte. `initialize` devuelve
    `instructions` (`SERVER_INSTRUCTIONS`) con el flujo de 10 pasos EN ORDEN. `workspace_status.
    capabilities` ya coherente con el perfil (E10-H08). Tests `perfil_readonly_sin_cambio`,
    `instrucciones_flujo` (orden de la espina, no "string no vacío"), `perfil_readonly_rechaza_cambio`
    (endurecido: invoca directamente las 3 de cambio bajo readonly con ids inexistentes → `-32602`;
    contraste con standard → `isError` de aplicación, distingue "rechazo por perfil" de "fallo por
    argumento") en `crates/lodestar-mcp/tests/mcp.rs`. `contracts/mcp.yml`: bloques `meta.perfiles` +
    `meta.protocolo.instructions`. Sin cambios en `core::types` (invariante #4; `Profile` es runtime, no
    wire → sin sync del espejo TS). Guardián de contrato: NO BLOQUEANTE (perfil de las 3 tools 1:1 con
    `CHANGE_TOOLS`). Juez ciego (seguridad escrutada, sin bypass): **APROBADA (2/2)**.
  - ✅ **E14-H04** — Benchmark funcional (`REFACTOR §17`) como suite e2e: `crates/lodestar-mcp/tests/
    benchmark.rs` ejercita los **15 escenarios** de §17 contra la superficie real (binario `lodestar-mcp`
    por stdio, JSON-RPC), un `#[test]` por fila (`bench_01`…`bench_15`) + el agregador
    `benchmark_15_escenarios`, con aserciones no-vacuas del resultado esperado (búsqueda por significado,
    create válido/rechazado, mover con 31 ops en un plan, borrar referenciado → `INBOUND_LINKS_EXIST`,
    `REVISION_CONFLICT`, 5 conceptos en un changeSet, relación inválida → `RELATION_CONSTRAINT_VIOLATION`,
    `apply_fix` sobre REL-TARGET, diff semántico, revert, crash+durabilidad, `PERMISSION_DENIED` fuera de
    writable, ref de código inexistente → `exists:false`, edición manual inválida → `knowledge_check`).
    Usa los códigos de error REALES del motor (documentados como divergencia consciente frente a los
    idealizados de §17). El escenario de crash reutiliza `recovery_sin_parciales` (E13-H06) + durabilidad
    e2e tras reabrir. **El benchmark destapó un hueco real de seguridad (invariante #3) que la fase verde
    cerró**: `Workspace::validate_staging` medía solo `analyze().hard_fail` (OKF) y NO la conformidad
    schema-driven → `change_apply` podía **publicar** un resultado con `SCHEMA-*`/`REL-*` err reportando
    `conformant:true` (mientras `knowledge_check`/`lodestar check` lo dirían no-conforme). Arreglo: el gate
    usa ahora `plan::validate_result(&bundle, &schema)` — la MISMA función del core que `change_plan` usa
    para `canApply` (OKF `per_file` + `validate_schema` + `validate_relations`, cuenta solo `err`,
    `conformant == errors==0`) — así el gate transaccional y `change_plan`/`knowledge_check` convergen por
    construcción, no por lógica duplicada. Corre ANTES de backup/journal/publish (no toca el canónico).
    Layering intacto (`lodestar-workspace`→`core`, NO `app`; schema cargado con `WorkspaceSchema::load`).
    Sin regresión (E13-H11 regen, recovery con failpoints, 44 MCP verdes). Juez ciego (equivalencia del
    gate escrutada): **APROBADA CON RESERVAS (3/3)**. **Reserva menor registrada** (preexistente, dirección
    segura, sin trigger práctico): `plan::validate_result` aplana TODO `per_file` (incluye reservados
    `index.md`/`log.md`) mientras `knowledge_check` itera solo `concepts` → el gate puede ser *más*
    estricto que `knowledge_check` sobre `OKF-CONFLICT` de un reservado (nunca menos: dirección segura;
    y el `index.md` de staging se regenera limpio, sin trigger real). El delta del arreglo (schema+rel)
    apunta solo a paths de `concepts`, alineado con `knowledge_check`.
  - ✅ **E14-H05** — Métricas de evaluación y presupuesto de escala: **historia de composición/regresión**
    (cero producción) — arnés de medición sobre fixture sintética de ~10k conceptos generada en runtime
    (tempdir, nada committeado). `crates/lodestar-app/tests/escala.rs`: `bench_search_payload_acotado`
    (10k conceptos → `knowledge_search` acota el payload: `SearchResult` no tiene `body`, expone `snippet`
    de 160 chars; aserción no-vacua con un centinela al final de cada cuerpo, fuera de la ventana del
    snippet, que NO debe viajar en la respuesta serializada; + cota de payload como proxy de tokens;
    latencia registrada, sin umbral duro — ~8s en debug para 10k, O(n)) y `bench_concurrencia_segura`
    (dos `change_apply` concurrentes → exactamente UNO gana; el perdedor recibe `WRITE_CONFLICT`
    —observado— o `PLAN_STALE`, ambos rechazan limpio ANTES de publicar; determinista no-flaky por el lock
    exclusivo `O_CREAT|O_EXCL` de E13-H02 que serializa `apply_transaction` + reverify optimista bajo el
    lock; asevera integridad: un solo `.md`, revisión coherente). Juez ciego: **APROBADA (2/2)**,
    no-vacuidad y determinismo confirmados. Las mediciones adicionales del alcance
    (`graph_query`/`impact_analyze`/`change_plan`/tiempo de crash-recovery) no tienen criterio testeable
    propio (umbrales orientativos, gate opcional que no bloquea v2); registrables por `eprintln!` si se
    desea.
  - ✅ **E14-H06** — Retirada de la superficie heredada (10 tools heredadas → 10 objetivo): el "único
    rewrite" que anticipaba `mcp.yml §15`, ahora que todos los reemplazos existen y el benchmark (E14-H04)
    demostró que las nuevas cubren los 15 escenarios. `crates/lodestar-mcp/src/tools.rs` retira de `list()`
    y del `match` de `call()` las 10 heredadas (`query`, `conformance_check`, `find_backlinks`,
    `find_orphans`, `find_dangling`, `neighborhood`, `create_concept`, `update_frontmatter`,
    `generate_index`, `generate_tag_indexes`) + helpers muertos (`rel`/`write_outcome_json`/`parse_patch`/
    `json_to_yaml`) e imports huérfanos. Superficie resultante: EXACTAMENTE las **10 objetivo**
    (`workspace_status`, `knowledge_search`, `knowledge_get`, `schema_inspect`, `graph_query`,
    `impact_analyze`, `knowledge_check`, `change_plan`, `change_apply`, `change_revert`). Invocar una
    heredada → `-32602` (nombre de tool desconocido = parámetro inválido; `tools/call` sigue siendo método
    válido; convención coherente con la retirada de git en E9). `contracts/mcp.yml` reescrito: `tools:`
    lista solo las 10; las heredadas movidas a `§15` como RETIRADA en E14-H06 con su reemplazo semántico;
    recuentos narrativos → 10. **RETIRA EXPOSICIÓN, NO CAPACIDAD**: la mecánica de dominio sigue viva
    (dormida, como el vcs) en `lodestar-workspace` (`backlinks`/`neighborhood`/`query`/`create_concept`/
    `merge_frontmatter`/`generate_index`/`generate_tags`); la CLI mantiene `index`/`tags`/`check`; cero
    cambios en `core`/`store`/`workspace`/CLI/UI. Sin cambios en `core::types` (invariante #4). Tests
    `tools_list_solo_objetivo` (conjunto exacto de 10) + `tool_heredada_retirada` (las 10 → `-32602` sin
    ejecutar) en `crates/lodestar-mcp/tests/mcp.rs`; el autor migró/retiró los tests del contrato viejo a
    sus equivalentes de las tools objetivo (cerrando el hueco de `dangling` con `graph_dangling`). Guardián
    de contrato: **LIMPIO** (1:1 `list()`↔`call()`↔`mcp.yml`). Juez ciego: **APROBADA (2/2)**, capacidad
    conservada verificada.
  - **Cierre de E14 y del giro headless**: el motor queda medido, conviviendo con código sin poseer git ni
    el editor, y con la superficie MCP convergida a las **10 tools objetivo** de `§19.6`. E9–E14 completas.
  - **Pendiente al cierre de E14**: limpieza final de superficie `mcp.yml` → 10 tools objetivo (retirar
    `query`/`conformance_check`/`find_*`/`neighborhood`/`create_concept`/`update_frontmatter`/
    `generate_*`), descopada aquí desde E12/E13.

---

## Migración a workspaces Markdown universales (E15–E22) — EN CURSO

> Rama `refactor/markdown-universal`. Diseño ratificado: `ARCHITECTURE.md §20` (2026-07-23; fuente:
> `docs/REFACTOR_PHASE_2.md`). Lodestar deja de exigir OKF y opera sobre cualquier red de `.md` de un
> proyecto. **v0.3.0 será incompatible con v0.2.x**; `v0.2.0` queda como última versión OKF.

| Épica | Estado | Detalle |
|---|---|---|
| **E15** Workspace universal | 🟡 En curso | Retirada de vcs/generadores/init-zip/prototipo · raíz = `cwd` · descubrimiento recursivo · config opcional. |
| **E16** Modelo documental genérico | ⚪ Pendiente | `ParsedFrontmatter` YAML arbitrario · sin ficheros reservados · título derivado · patch quirúrgico · diagnósticos mínimos · `Concept`→`Document`. |
| **E17** Enlaces y grafo universal | ⚪ Pendiente | Parser de enlaces (`pulldown-cmark`) · `LinkTarget` · diagnósticos de enlace · `Analysis` nueva · superficie de grafo. |
| **E18** Store v2 | ⚪ Pendiente | DDL nuevo, metadata anidada, links genéricos, cold rebuild, paridad core/store. |
| **E19** Lenguaje de consulta | ⚪ Pendiente | Parser · AST · type checking · namespaces · filtro JSON equivalente. |
| **E20** Inspección y validación genéricas | ⚪ Pendiente | `metadata_inspect` (retira `core::schema`) · política `rejectNewErrors`/`allowExistingErrors`. |
| **E21** Contrato MCP y transacciones genéricas | ⚪ Pendiente | Contrato nuevo · 8 operaciones universales · selecciones masivas por consulta. |
| **E22** Migración y limpieza pública | ⚪ Pendiente | `migrate-from-okf --dry-run` · docs · README · publicación incompatible. |

### E15 — Workspace universal

- ✅ **Puerta de diseño** — `ARCHITECTURE.md §20` escrita y ratificada (adenda de 14 subsecciones;
  notas de supersesión en §4, §10 y §19). Épicas E15/E16/E17 descompuestas en `requirements/`.
- ✅ **E15-H05** — Fixtures de workspaces Markdown arbitrarios (`crates/lodestar-fixtures`):
  `arbitrary()` (raíz + 3 niveles, enlaces cruzados en ambos sentidos, sin `index.md` ni
  frontmatter), `with_edge_cases()` (espacios, `%20`, oculto, mismo basename en dos árboles,
  capitalización errónea, código, externo, anchor, inexistente, escape), `materialize()` y
  `materialize_disk_only()` (no UTF-8, sobre el límite, symlink, `.gitignore`, `.lodestarignore`).
  **Aditivo**: los bundles OKF heredados siguen vivos hasta que E16/E17 retiren a sus consumidores.
  4 tests.
- ✅ **E15-H01** — `lodestar-vcs` **borrado** (crate, `git2`, `build.rs`, tests). Fuera del
  `Workspace`: campos `vcs`/`identity`, `Vcs::discover`/`init`, `set_identity`, `has_vcs`,
  `init_vcs`, `init_bundle`, `commit`/`restore`/`switch`/`merge`/`create_branch`/`branches`/
  `vcs_log`/`last_conforming`/`conformance`/`conformance_of`/`install_hooks`/`push`/`pull`/
  `diff_working`/`analyze_rev`/`analyze_staged`, `CommitOutcome`/`MergeReport`, y las variantes
  `Vcs`/`NoVcs`/`RepoBusy` de `WorkspaceError` con su `From<VcsError>`. Fuera de `core::types`:
  `Sha`/`Author`/`CommitRow`/`CommitConformance`/`RepoState`/`Branch`/`SyncKind`/`SyncOutcome` y
  `CoreError::InvalidSha`. Store: tabla `commit_conformance` (DDL, probe, accesores) fuera y
  **`USER_VERSION` 1 → 2** (una cache v0.2 se detecta antigua y se reconstruye limpia). `identity`
  fuera de `Config`/`WorkspaceConfig`. **Conservado** `workspace/src/gitignore.rs` (texto plano).
  Tests: `abre_sin_repo_git`, `cache_v2_se_reconstruye`.
- ✅ **E15-H02** — Generadores **borrados**: `core::generate`, `Bundle::gen_index`/`gen_tag_indexes`,
  `Workspace::generate_index`/`generate_tags`, subcomandos `index`/`tags` de la CLI y el **exit code
  4** (drift), y la auto-regeneración de E13-H11 dentro de `apply_transaction` (el apply publica
  exactamente el resultado del change set). `Mutation` se conserva (motor transaccional). Tests:
  `help_sin_generadores`, `index_es_uso`, `apply_no_regenera_indices` (sustituye a `regen.rs`).
- ✅ **E15-H03** — `init`/`export`/`import` **borrados** de la CLI (clap + dispatch), con
  `Bundle::export_zip`, `CoreError::Export`, la dependencia `zip` (workspace, core y cli) y
  `crates/lodestar-cli/src/bundle_io.rs` entero (quedó sin consumidores: `check` va por `App` y
  `reindex` por `Workspace`). La CLI queda en `check` + `reindex`. Tests:
  `help_solo_check_y_reindex`, `init_es_uso`.
- ✅ **E15-H04** — Prototipo retirado como spec: `crates/lodestar-core/tests/differential.rs`
  borrado y el CI sin node/`npm ci`. `CLAUDE.md`, `requirements/README.md` y `docs/WORKFLOWS.md`
  declaran ahora `docs/REFACTOR_PHASE_2.md` + `ARCHITECTURE.md §20` como spec de comportamiento y
  `prototype/` como referencia histórica de v0.2.x (el directorio **se conserva**). El job
  `core-purity` añade `zip` a la lista prohibida y un guard nuevo verifica que
  `cargo tree --workspace` no muestre `git2`/`lodestar-vcs`/`zip`.
- ⚠️ **Cobertura perdida a propósito en el bloque de retirada** (queda registrada, no es deuda a
  saldar): (1) al morir `import` desaparece la única superficie de **zip-slip**, así que esa mitad
  del invariante #6 deja de ser alcanzable — el chokepoint `RelPath` sigue testado para absolutas y
  `..`; (2) `tags_ordenados_con_locale_compare` era el único test de la colación `localeCompare` de
  tags, pero su única superficie observable era `gen_tag_indexes`: sin generador no hay dónde
  observarla (`locale_cmp` sobrevive en `core::model`, hoy sin consumidor — candidato a borrarse en
  E16 si sigue huérfano).
- 📌 **Punteros de proceso actualizados**: `.claude/agents/*` (autor-tests, implementador,
  historiador, planificador), `.claude/README.md`, `DECISIONES.md §9` y
  `requirements/paridad-auditoria.md` daban por vivo el arnés diferencial y el `npm ci` de
  `prototype/harness/`; ahora lo declaran retirado en `E15-H04`.
- ⚖️ **Juez ciego (H01–H04)**: **APROBADA CON RESERVAS**, 11/11 criterios cumplen. Hallazgos
  corregidos después:
  - *Isla de código muerto*: `Workspace::apply_mutation` quedó sin llamadores (sus consumidores eran
    `generate_index`/`generate_tags`/`switch`/`merge`/`restore`, todos borrados). Borrados él,
    `ApplyReport`, `core::types::Mutation` y `cache_remove`. La nota de "fuera de alcance" de
    E15-H02 —que justificaba conservar `Mutation` porque «lo usa el motor transaccional»— era
    **factualmente falsa** tras retirar la auto-regen; corregida en la épica.
  - *Contrato desalineado*: la semántica normativa de `change_apply`/`change_revert` en
    `contracts/mcp.yml` seguía anunciando la auto-regeneración de `index`/`tags`.
  - *Menores*: exit code 4 aún en la tabla de `CLAUDE.md`; `ignore` huérfano en `lodestar-cli`;
    doc-comments de `publish.rs`/`staging.rs` justificando la escisión `publish`/`publish_result`
    por la auto-regen (la escisión **se conserva**: vale por sí sola, se publica exactamente el mapa
    que se validó); `RELEASING.md` publicando `lodestar-vcs` y omitiendo `lodestar-app`.
  - *Hueco preexistente, no regresión*: `reindex` no tiene ningún test que lo ejecute, y ahora es la
    mitad de la superficie de la CLI. Pendiente.
- ✅ **E15-H06** — **La raíz del workspace es el `cwd`**. El MCP pierde el gate que abortaba con
  exit 3 si no había `index.md`/`.lodestar/`: cualquier directorio es un workspace. `parse_args`
  pasa a `[--root <dir>] [--profile …]` — **el argumento posicional se retira** (v0.3 es
  incompatible; un argumento no reconocido sale con exit 2 y `USAGE`, en vez de arrancar en silencio
  sobre el cwd equivocado). La raíz se **canonicaliza una sola vez al arrancar** y no cambia en toda
  la sesión (`§20.5`). En la CLI, `resolve_root` deja de ascender por los ancestros. Contrato:
  `meta.arranque` reescrito y `meta.paths` **nuevo** en `contracts/mcp.yml` (absolutas y `..` se
  rechazan vía `RelPath` con `isError` en el result, nunca error de protocolo, y sin tocar disco).
  Arnés migrado (`.arg(dir)` → `.arg("--root").arg(dir)`) en los 3 helpers que cubren ~82
  invocaciones, sin tocar ninguna aserción. Tests: `arranca_en_directorio_arbitrario`,
  `root_explicito_gana`, `cli_no_asciende` + las guardas `rechaza_absoluta`/`rechaza_escape` (con
  cebo real fuera de la raíz, en lectura y en escritura). Borrado el obsoleto
  `directorio_no_bundle_sale_con_3`, que era la negación literal de la historia. **232 tests**.
  - **Verificado a mano**: `cd` a un proyecto de 7 `.md` repartidos en `docs/`, `packages/*/docs/`,
    `knowledge/roadmap/` y la raíz, **sin** `index.md`, `.lodestar/` ni frontmatter → el servidor
    arranca, `workspace_status` reporta los 7, y `graph_query` resuelve el enlace raíz →
    `packages/api/docs/endpoints.md` y el de vuelta `../../../README.md` **en el mismo grafo**. Es
    el `§Resultado esperado` de `docs/REFACTOR_PHASE_2.md`.
- ✅ **E15-H07** — **Descubrimiento recursivo universal**. Módulo `discovery` (`DiscoveryPolicy`,
  `Discovered`, `discover`, `case_collisions`, `rel_path_from`) que sustituye a `io::load_bundle` en
  sus **7 llamadores**, por un punto de inyección único (`Workspace::discovery_policy` +
  `discover_files`) para que `bundle()`, `workspace_revision()` y el motor transaccional vean el
  mismo inventario. 5 códigos nuevos en `CheckCode` (`DOC-NOT-UTF8`, `DOC-TOO-LARGE`,
  `PATH-NOT-UTF8`, `SYMLINK-UNSUPPORTED`, `LINK-CASE-MISMATCH`), todos `Warn`. Determinismo
  reforzado más allá de lo pedido: `parents(false)` + `git_global(false)` + `git_exclude(false)`, de
  modo que el inventario dependa solo del árbol bajo la raíz. `io::load_bundle` borrado. 10 tests.
  - **Corrección durante la historia**: la política excluye **`.lodestar/` entero**, no solo
    `runtime/`. Un `.md` ahí sería nodo del grafo y escribible pero **ciego al control optimista**
    (`workspace_revision` excluye todo `.lodestar/` por D5, y no puede dejar de hacerlo: `StagingDir`
    materializa ahí copias `.md` de los documentos que está guardando — si contara,
    `reverify_base_revision` fallaría *a causa del apply en curso*). `§20.5` enmendada.
- ⚖️ **Juez ciego (H06 + H07)**: **RECHAZADAS** ambas, con 3/4 y 7/9 criterios cumplidos. Dos
  bloqueantes reales:
  - **H06** — `rechaza_absoluta` **falla en `windows-latest`**: el cebo (`C:\Users\…`) se interpola
    crudo en un literal de cadena JSON y `\U`/`\A`/`\T` no son escapes válidos → el servidor
    responde `-32700` y el test panica. Defecto de arnés, no de producto.
  - **H07** — **regresión silenciosa**: los patrones de `.gitignore`/`.lodestarignore` **a nivel de
    fichero** dejaron de aplicarse. `include: ["**/*.md"]` entra como whitelist del `Override`, y en
    el crate `ignore` los overrides tienen precedencia absoluta y cortocircuitan. Los patrones de
    **directorio** siguen funcionando por accidente (el override no aplica whitelist a directorios,
    así que el directorio se poda antes de descender) — y por eso los dos tests que demuestran esos
    criterios pasaban **por la razón equivocada**.
  - Otros: symlinks de **directorio** sin diagnóstico (MAYOR-2); `.ignore` siempre aplicado y no
    desactivable (`WalkBuilder::ignore` vale `true` por defecto y nunca se toca); `**/*.md` es
    case-sensitive, así que `README.MD` no se descubre; `rel_path_from` normaliza `\`→`/` también en
    Unix, donde `\` es legal, y un `a\b.md` puede enmascarar al `a/b.md` real. Los tres últimos son
    heredados de `io::load_bundle`, no regresiones.
  - **MAYOR-1 → historia nueva E15-H09**: `assert_writable` no consulta la política de
    descubrimiento, así que se puede escribir en paths excluidos del inventario **y** de la revisión.
    `REFACTOR_PHASE_2 §8` lo prohíbe explícitamente.
- ✅ **E15-H09** — **La política de escritura respeta el descubrimiento** (cierra E15). Pieza nueva
  `discovery::exclusion_reason`: la versión "una ruta suelta, sin recorrer el árbol" de `discover`,
  necesaria porque el destino de un `create`/`move` **todavía no existe**. Reproduce el mismo orden
  de precedencia reusando los constructores de `discover`, de modo que un «sí» significa literalmente
  «ese path, una vez escrito, saldrá en el inventario». Se rechaza en `change_plan` **y** en
  `assert_writable` (apply + revert): lo segundo no es redundante, porque el descubrimiento es estado
  del árbol y un `.gitignore` que aparece entre plan y apply no mueve la `WorkspaceRevision` ni
  invalida el `planHash`. El escenario 13 del benchmark sobrevive porque `change_plan` llama solo a
  `assert_discoverable`, no a `assert_writable` entero. Cruce documentado: cuando `writableRoots`
  permite lo que el descubrimiento excluye, **manda la exclusión** (es lista de permiso, no de
  habilitación). 4 tests. **257 tests · E15 COMPLETA (H01–H09).**

### E16 — Modelo documental genérico

- ✅ **E16-H01** — **Frontmatter YAML arbitrario**. La cirugía más ancha de la migración: ~95 puntos
  en 13 ficheros. Muere `Frontmatter` (7 campos tipados), `KNOWN_FM`, `known_null`, `as_pairs`,
  `js_string`, `dump_frontmatter`, `FmError::Missing` y `types::ParsedFile` (jamás construido desde
  E1); cae `indexmap` como dep directa del core. Nace
  `ParsedFrontmatter { value, raw, span }` con `FieldPath` (newtype de **segmentos**, con `parse`
  para dot-notation y `from_segments` para claves que contienen un punto) como **única verdad de
  acceso a metadata**, que reutilizarán E18/E19/E20. `split_front` reescrito por bytes: corrige el
  bug por el que `---\n---\n` se reportaba como frontmatter *sin cerrar*. 262 tests.
  - **Aviso registrado para E19** (`§20.8`): las comparaciones deben ir sobre `get`, nunca sobre
    `get_text` — construirlas sobre este último reintroduciría la coerción implícita **sin que
    ningún test lo notara**.
  - **Defecto de fixtures del autor, corregido por él**: las continuaciones de línea de Rust (`\`)
    se comen la indentación, así que su YAML anidado llegaba aplanado. Auditadas las 45 apariciones
    del patrón en los 6 ficheros de test de la migración: ninguna otra estaba rota — E16-H01 es la
    primera historia cuyas fixtures necesitan YAML **anidado**.
