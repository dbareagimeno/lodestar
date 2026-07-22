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
cargo run -p lodestar-mcp -- <bundle>                  # servidor MCP por stdio
cd frontend && npm install && npm run build            # frontend Svelte 5 → dist/
# Escritorio (requiere libwebkit2gtk-4.1-dev, libsoup-3.0-dev, …):
cargo run -p lodestar-tauri                            # app de escritorio (Tauri v2)
```

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

## Giro a motor headless de integridad semántica (E9–E14) — EN CURSO

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
  - ✅ **E9-H07** — Documentación de producto reposicionada: `README.md`/`CLAUDE.md` describen el
    posicionamiento como motor headless de integridad semántica, citan `ARCHITECTURE.md §19`,
    listan `lodestar-app` en el mapa de crates y marcan git como capacidad dormida y la UI como
    congelada. Este bloque de `IMPLEMENTATION_STATUS.md` refleja E9 completa.
- **E9 — COMPLETA** (H01–H07, las 7 historias de la fase 0).
- **E10 — EN CURSO** (esquemas + lectura headless):
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
  - ⏳ E10-H08–H13 (5 tools de lectura + outputSchema/mcp.yml) pendientes.
- **E11–E14: pendientes** (grafo e impacto · planificación · publicación recuperable ·
  integración software + evaluación — `ARCHITECTURE.md §19.8`).
