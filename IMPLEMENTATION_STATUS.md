# Estado de implementación

> Mapea las épicas/historias de [`requirements/`](requirements/) a su estado real en esta rama.
> Construido en el **orden de fases ratificado** (`ARCHITECTURE.md §14`), validando con tests en cada fase.
>
> **Resumen** (actualizado al cierre de `E25`/`E26`): el repo es un **motor headless de integridad
> semántica** sobre workspaces Markdown universales (`ARCHITECTURE.md §20`). Las épicas **E0–E8**
> (fundacionales), **E9–E14** (giro headless), **E15–E22** (migración de OKF a Markdown universal),
> **E23** (cierre: defectos hallados en la revisión de la PR #17), **E24** (cierre de los defectos
> que la revisión de la v0.3.0 destapó tras publicarla) y **E25/E26** (endurecimiento del camino de
> escritura y de la superficie de errores) están **completas**.
> Backend: `core` puro + `store` SQLite/FTS5 con paridad SQL==core + `workspace` (único escritor,
> transaccional y **recuperable con recibo**) + `app` (servicios de caso de uso) + las dos fachadas
> `cli` y `mcp`. Suite en verde (+ los de crash-recovery tras `--features test-failpoints`, que el CI
> corre desde E23-H06); `clippy -D warnings` y `cargo doc -D warnings` limpios; pureza del core
> verificada por CI. **El recuento exacto de tests lo fija la nota de release** (E24-H18:
> `cargo test --workspace -- --list | grep -c ": test$"`); fijado en la **v0.5.0**: **541 tests**
> (eran 486 al cerrar E24, antes de las 11 historias de E25/E26). En `develop` van **542**: +1 por la
> guardia del `outputSchema` (ver «Defectos posteriores a E27» al final); la cifra de la v0.5.0 no se
> reescribe — la fija su release.
>
> **Ya no forman parte de este repo**: la app de escritorio (Tauri + Svelte, movida a
> `experimental/ui-desktop` con el giro headless), el crate `lodestar-vcs` y `git2` (borrados en
> `E15-H01`), los generadores `init`/`index`/`tags`/`export`/`import` (`E15-H02`/`H03`), el arnés
> diferencial JS-vs-Rust (`E15-H04`) y el `Envelope<T>`/`ErrorEnvelope` de `lodestar-app`
> (`E29-H11`, `decisiones §16(b)`: capacidad construida en E10-H01/H02 sin consumidor real — el wire
> siempre fue `structuredContent`/exit codes directos). Las secciones de E0–E8 de más abajo
> conservan esa terminología como **historia del proyecto**; la autoridad viva es
> `ARCHITECTURE.md §20`.
>
> Lo pendiente está en [`decisiones/`](decisiones/README.md): fechas en el lenguaje de consulta (§12),
> el **store sin consumidor** (§14, abierto en E23-H16). §12 y §13 se cerraron en E23-H14.

## Cómo correrlo

```bash
cargo test --workspace --locked                       # 541 tests (v0.5.0)
cargo test -p lodestar-workspace --features test-failpoints --locked   # crash-recovery (E13-H06)
cargo test -p lodestar-app --features test-failpoints --locked         # ventana de publicación (E25-H01)
cargo run -p lodestar-cli -- check [--path <dir>]     # la puerta de CI (exit 0/1/2/3)
cargo run -p lodestar-cli -- reindex                  # reconstruye .lodestar/index.db
cargo run -p lodestar-cli -- migrate-from-okf --dry-run
cargo run -p lodestar-mcp [-- --root <dir>] [--profile readonly|standard]   # MCP por stdio
```
(La app de escritorio —`frontend/` + `src-tauri/`, binario `lodestar-desktop`— se retiró de `main`
a la rama `experimental/ui-desktop`; ya no se construye ni se ejecuta desde este repo headless. Los
subcomandos git de la CLI —`log`/`last-conforming`/`branch`/`switch`/`merge`/`hooks`— se retiraron en
`E9-H02` y su mecánica se borró en `E15-H01`.)

## Estado por épica (E0–E8) — **histórico**

> ⚠️ Esta tabla describe las épicas fundacionales **tal como se construyeron**, y su vocabulario es
> el de v0.1/v0.2 (OKF, bundle, concept, conformidad, git, UI). Varias de las capacidades que lista
> **ya no existen**: `lodestar-vcs` y `git2` se borraron en `E15-H01`, los generadores
> (`index`/`tags`/`export`/`init`) en `E15-H02`/`H03`, el arnés diferencial en `E15-H04`, la UI se
> movió a `experimental/ui-desktop`, y las 13 tools de E7 convergieron a las **10** de `§20.10`.
> Se conserva como historia del proyecto. **El estado vigente son las secciones de E9–E14 y E15–E23**
> de más abajo; la autoridad de diseño es `ARCHITECTURE.md §20`.

| Épica | Estado | Detalle |
|---|---|---|
| **E0** Scaffolding | ✅ Hecho | Cargo workspace con 7 crates + direcciones del §3; `#![forbid(unsafe_code)]` en core; fixtures; CI (fmt/clippy/test + frontend); frontend Svelte/Vite. |
| **E1** `lodestar-core` | ✅ Hecho | Contrato de tipos congelado, modelo, conformidad (15 checks + OKF-CONFLICT), analyze, query, grafo, generadores, export, diff. **Arnés diferencial JS-vs-Rust (H18, §12)**: 6 fixtures corren las funciones puras del prototipo (vía node) y comparan con el core — la red de paridad. La auditoría halló y corrigió **6 divergencias** (NFC en slugs, orden numérico de tags con `sort_paths_cmp`, `null` en `yaml_is_empty`/`fm_present`, aristas a reservados en el grafo, orden de aparición de extras vía `IndexMap`). 22 + 6 tests. |
| **E2** `lodestar-cli` | ✅ Hecho | `check` (humano/--json/--sarif), `index`/`tags` (--check→exit 4), `export`, `init`; exit codes congelados. 8 tests. |
| **E3** `lodestar-store` | ✅ Hecho | DDL dueño único (`files`/`links`/`tags`/`diagnostics` + FTS5 + `commit_conformance`), cold rebuild, watcher `notify-debouncer-full` con **gate por hash blake3**, síntesis SQL (backlinks/orphans/dangling/blast-radius CTE), FTS5 con escapado, bus `IndexEvent` (crossbeam), trait `ConceptStore`. **13 tests**: paridad SQL==core, property incremental==core (120 ediciones), watcher en vivo, FTS. |
| **E4** `lodestar-vcs` | ✅ Hecho | libgit2 local + red por binario `git` + **resolve_rev**, **staged_files**, **switch** (sin tocar working tree), **merge** (3-vías a nivel de árbol con marcadores + `MERGE_HEAD`), **install_hooks**, **tree_oid**. Cache de conformidad por tree-oid en el store, cableada en la workspace. **12 tests**. |
| **E5** `lodestar-workspace` | ✅ Hecho | Handle unificado, único escritor, snapshot, commit/restore con checkpoint, switch/merge, conformidad cacheada por tree-oid, config (`lodestar.toml`), y **bus de eventos en vivo** (`open_live`/`enable_cache`/`subscribe`) con **update optimista** de la cache tras cada escritura. **12 tests**. |
| **E6** Tauri + frontend | ✅ Hecho | **Fachada Tauri v2** real: comandos congelados sobre `Workspace` + estado del bundle + forwarder del bus `IndexEvent` → evento `bundle:changed` (UI en vivo). Binario `lodestar-desktop` compila; CI de Rust instala webkit y construye el frontend antes. **Frontend Svelte 5 funcional**: layout de 3 columnas colapsables, árbol filtrable, editor multi-escritor con validación y diagnósticos localizados, panel de enlaces, **isla imperativa del grafo** (`createStarMap`, SVG+rAF, sin `{#each}`), modo **Cambios** (diff + commit). `npm run check`/`build` verdes. Pulido en [`decisiones §2`](decisiones/02-port-ui-prototipo.md). |
| **E7** `lodestar-mcp` | 🟢 Parcial | 13 tools sobre la workspace + bucle JSON-RPC por stdio (stdout puro). **Golden cross-fachada** (tool==workspace) + e2e. **5 tests**. Pendiente: transporte `rmcp` oficial + resources (ver [`decisiones §3`](decisiones/03-transporte-mcp-rmcp.md)). |
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
  [`decisiones §4`](decisiones/04-generacion-dts-ts-rs.md)).

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
  notarización diferida — ver [`decisiones §1`](decisiones/01-build-fachada-escritorio.md)). `bundle.active = true` y los iconos
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

- **Core puro**: `lodestar-core` no declara `tauri`/`rusqlite`/`notify`/`tokio`/`git2`/`zip`;
  `#![forbid(unsafe_code)]`. Verificado por el job `core-purity` del CI.
- **Una sola verdad computada**: `lodestar check` y `knowledge_check` juzgan con el mismo motor
  (`E23-H01`; hasta entonces divergían cuando `.lodestar/config.yaml` tenía sección `validation`).
- **Un solo contrato de tipos**: definido una vez en `core::types`. **Sin espejo TS**: desapareció al
  retirar la UI a `experimental/ui-desktop`, y con él la nota de ts-rs.
- **RelPath**: newtype validado; único chokepoint de path-traversal (tests de absolutas/`..`).
- **Único escritor**: la workspace escribe `.md` atómico (temp+rename); nadie más escribe. Desde
  `E25-H01`, además, **lo publicado es exactamente lo respaldado**: ninguna escritura del canónico
  esquiva `assert_writable`, la copia de recuperación y el journal, y desde `E25-H05` el **borrado**
  es tan durable como la escritura.
- **Crash-recovery**: un crash a mitad de la publicación nunca deja un `.md` parcial. Verificado por
  los tests tras `--features test-failpoints`, que el **CI ejecuta desde `E23-H06`**. Desde
  `E25-H04`/`E25-H05` la garantía es más fuerte: **si el canónico cambió, existe el recibo que lo
  deshace** — en el apply y en el revert, también tras `SIGKILL`. Y desde `E25-H02` las copias de
  recuperación se **verifican por huella** antes de restaurarse: una copia corrupta no se publica
  como si fuera el original, va a cuarentena con `RECOVERY_FAILED`.
- **Propiedad del lock** (`E25-H06`): el lockfile lleva token de propiedad e identidad de máquina, de
  modo que un `Drop` no puede liberar el lock de otro dueño ni se reclama el de un pid vivo.
- ~~**git vocabulario directo**~~ — retirado con `lodestar-vcs` en `E15-H01` (`§20.13`).

## Próximos pasos (ver [`decisiones/`](decisiones/README.md))

> **Repriorización conjunta del 2026-08-02** — el orden de trabajo vigente vive al final de
> [`decisiones/README.md`](decisiones/README.md) y es: **(1)** épica de honestidad de superficie
> (§19a/b, §18 vinculante, §16f, §16e, §15, §16g, §16b) · **(2)** ciclo de higiene (§16 i/j/l) ·
> **(3)** épica de evidencia = banco de pruebas §9 + dogfooding, que **cierra §14 con datos** ·
> **(4)** renombrado del proyecto (`§20`, nuevo, alcance total incluido `.lodestar/`; congela la
> firma de binarios y crates.io) · **(5)** comillas en el lenguaje (`§21`) · **(6)** ghosts (`§10`).
> En esa pasada **`§16` se disolvió** en sus dueños reales y **`§5` se cerró**. La lista de abajo es
> anterior y se conserva como registro de lo que estaba pendiente al cerrar E23/E24.

**E23 está cerrada, así que nada bloquea el merge de v0.3.0.** Lo que sigue está abierto a criterio
del usuario:

1. **`decisiones §14`** — el store (épica E18) no tiene consumidor: ninguna tool lee de SQLite y
   `document_set()` reparsea la base entera en cada llamada. Decidir si se conecta, se acota o se
   retira. Va con la deuda hermana: el walker del store **no aplica la `DiscoveryPolicy`**, así que
   la paridad core↔store solo se sostiene bajo política por defecto — inocua mientras nadie lea el
   store, bug real en cuanto se conecte.
2. ~~`decisiones §12` (fechas) y `§13` (`Conformant → Valid`)~~ — **cerradas en E23-H14**: las
   fechas se declaran lexicográficas por escrito, y el catálogo de errores se abrió la única vez
   para completar la pareja de `§20.3`.
3. **`docs/history/PROPUESTA_CLI.md`** — la CLI como gestor de KB (hoy solo es puerta de CI). Pendiente de
   `/planificar` en una PR posterior. Su condición de entrada dura —que existan tests de
   concurrencia entre procesos— **ya se cumple** desde `E23-H09`.
4. **`docs/history/PROPUESTA_FIXES.md`** — reactivar los arreglos sugeridos (`Fix`/`apply_fix`, la op
   retirada en `E23-H11`). Condición de entrada: que existan productores de `Fix` que justifiquen la
   maquinaria del `fixId` direccionable entre revisiones.
5. **`decisiones §3`/`§9`** — `rmcp` oficial + resources cuando un cliente lo exija; gate de bench y
   threat model.

Deuda menor registrada, con dueño futuro: `Workspace::materialize_staging` es API pública que
escribe bajo `.lodestar/` **sin pasar por ninguno de los cuatro chokepoints** de `E23-H12`; hoy es
inofensiva porque no tiene llamador de producción, pero si se le da uno, tiene que pasar por un
chokepoint o convertirse en el quinto.

**Añadido al cerrar E25/E26**: la auditoría del camino de escritura y las reservas de sus jueces
ciegos dejaron **once puntos de deuda declarada** —lo que quedó explícitamente fuera de esa tanda,
cada uno con su origen— en la sección nueva [`decisiones §16`](decisiones/16-deuda-auditoria-e25-e26.md). Ninguno bloquea el
merge: van de sintaxis de *quoting* que hoy es ruidosa pero correcta, a capacidad construida sin
consumidor (`Envelope`, la cache SQLite), pasando por API pública no transaccional sin llamadores y
por la matriz de trazabilidad, que sigue **sin filas de E15–E24**.

## Giro a motor headless de integridad semántica (E9–E14) — COMPLETO

Refactor de `docs/history/REFACTOR.md`, diseño ratificado en `ARCHITECTURE.md §19` (supersede §13 en
superficie de producto; git queda como crate dormido) y `decisiones §0`. Descomposición en
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
    (`diag:blake3:` solo de datos del diagnóstico); `valid`/`summary` computados antes de
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
    all_checks; valid=errors==0 explícito) + `PlanPolicy{requireValidResult,allowWarnings}`
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
    aplica el gate estricto y limpia + `INVALID_RESULT` si no conforme. `WorkspaceError::
    InvalidResult`. Juez ciego: APROBADA (2/2).
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
    consume `full_analysis` y deriva `valid` con la misma regla del motor. Salida humana / `--json`
    (campo `valid` aditivo + `perFile` con los `SCHEMA-*`/`REL-*`) / SARIF (`ruleId` schema/rel)
    surfacean los diagnósticos del motor completo, no solo el veredicto. Exit codes CONGELADOS
    (`0`/`1`/`2`/`3`) intactos; `blocked` es superconjunto del anterior (nada que bloqueaba deja de
    hacerlo). Sin cambios en `core::types` (invariante #4; `valid` inyectado en la fachada). Tests
    `check_falla_schema`/`check_conforme_json`/`check_caza_edicion_directa` + surfacing
    `check_sarif_lista_schema`/`check_json_lista_schema` en `crates/lodestar-cli/tests/cli.rs`. Juez
    ciego (2 pasadas): la 1ª APROBADA CON RESERVAS (salida no surfaceaba schema/rel) → cerrada con
    micro-ciclo rojo→verde; la 2ª (historia completa, no-regresión MCP 41 tests verdes) **APROBADA
    (6/6)**. Hallazgos menores heredados (no bloqueantes): (1) `check` abre el `App` completo → puede
    materializar la cache `store` como efecto de un comando read-only (mismo camino que MCP, coherente
    con invariante #5); (2) `valid` juzga solo `concepts` mientras `gate_blocked` cuenta `Err` de
    todos los ficheros (p. ej. `index.md`) → un error solo en `index.md` da `valid:true` pero exit
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
    `instructions` (`SERVER_INSTRUCTIONS` [retirado en E29-H09: hoy `server_instructions(profile)`])
    con el flujo de 10 pasos EN ORDEN. `workspace_status.
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
    `valid:true` (mientras `knowledge_check`/`lodestar check` lo dirían no-conforme). Arreglo: el gate
    usa ahora `plan::validate_result(&bundle, &schema)` — la MISMA función del core que `change_plan` usa
    para `canApply` (OKF `per_file` + `validate_schema` + `validate_relations`, cuenta solo `err`,
    `valid == errors==0`) — así el gate transaccional y `change_plan`/`knowledge_check` convergen por
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

## Migración a workspaces Markdown universales (E15–E22) — COMPLETA

> Rama `refactor/markdown-universal`. Diseño ratificado: `ARCHITECTURE.md §20` (2026-07-23; fuente:
> `docs/REFACTOR_PHASE_2.md`). Lodestar deja de exigir OKF y opera sobre cualquier red de `.md` de un
> proyecto. **v0.3.0 será incompatible con v0.2.x**; `v0.2.0` queda como última versión OKF.

| Épica | Estado | Detalle |
|---|---|---|
| **E15** Workspace universal | ✅ Completa | Retirada de vcs/generadores/init-zip/prototipo · raíz = `cwd` · descubrimiento recursivo · config opcional (H01–H09). |
| **E16** Modelo documental genérico | ✅ Completa | `ParsedFrontmatter` YAML arbitrario · sin ficheros reservados · título derivado · patch quirúrgico · diagnósticos mínimos · `Concept`→`Document` (H01–H06). |
| **E17** Enlaces y grafo universal | ✅ Completa | `pulldown-cmark` en el core · `LinkTarget` · diagnósticos de enlace · `Analysis` nueva · grafo universal + cableado de `other_files`. |
| **E18** Store v2 | ✅ Completa | DDL nuevo, metadata anidada por field path, links genéricos, cold rebuild, paridad core/store. **Sin consumidor en el producto** → `decisiones §14`. |
| **E19** Lenguaje de consulta | ✅ Completa | Parser · AST único · type checking sin coerción · namespaces `document.*`/`graph.*` · filtro JSON equivalente. |
| **E20** Inspección y validación genéricas | ✅ Completa | `metadata_inspect` (retira `core::schema`) · política `rejectNewErrors`/`allowExistingErrors` · diagnósticos de descubrimiento cableados. |
| **E21** Contrato MCP y transacciones genéricas | ✅ Completa | 8 operaciones universales · selecciones masivas por consulta · `move` por span · `delete` con política explícita. |
| **E22** Migración y limpieza pública | ✅ Completa | `migrate-from-okf --dry-run` · README · v0.3.0 incompatible · e2e de la migración. |
| **E23** Cierre de la migración | ✅ Completa | Defectos de la revisión de la PR #17 · puerta de CI (failpoints) · e2e de sesión larga · schema de escritura · apertura hermética · descubribilidad · documentos. Ver [`epica-23`](requirements/epica-23-cierre-migracion.md). |

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
  historiador, planificador), `.claude/README.md`, `decisiones §9` y
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

- ✅ **E16-H02** — **Ningún nombre de fichero activa reglas especiales**. Mueren `FileKind`,
  `model::file_kind`/`is_reserved`/`concept_id`, `RelPath::is_reserved`/`concept_id`,
  `Bundle::root_okf_version`, la rama de reservados de `parse_file`, `Parsed::kind`,
  `validate_index`/`validate_log` (con ellas `OKF-IDX`/`OKF-LOG` se quedan sin productor), el check
  `ORPHAN`, el gating de fichero reservado de `query.rs` (con `is:reserved`) y el quirk de
  `graph_model`/`neighborhood` que descartaba las aristas a `index.md`/`log.md`. `compute_analysis`
  toma **todos** los `.md` como nodos. `Analysis` pierde `in_index`/`okf_version` y `orphans` pasa a
  `isolated` con la definición de `§20.7` (sin entrantes **ni** salientes); `Backlinks` pierde
  `index_refs`; `ConceptSummary.orphan` → `.isolated`; `is:orphan` → `is:isolated`.
  - **Cara del store**: el DDL pierde `files.kind` y `links.src_is_index` (`USER_VERSION` 2 → 3, la
    cache se reconstruye sola), `Store::orphans`/`in_index` → `Store::isolated`, y los enlaces se
    extraen SIEMPRE del cuerpo. La paridad SQL == core sigue verde.
  - **Frontera MCP sincronizada** (`contracts/mcp.yml`): `graph_query.operation` `"orphans"` →
    `"isolated"` **sin alias** (v0.3 es incompatible por diseño; un alias devolvería otra cosa bajo
    el mismo nombre), `workspace_status.counts.orphans` → `counts.isolated`, y `formatVersion` pasa
    a constante — el motor ya no lee `okf_version` del `index.md` raíz (`§20.13`).
  - **Efecto de segundo orden asumido**: mientras `OKF-TYPE` siga vivo (muere en E16-H05), un
    `index.md` sin `type` es un hard-fail. Los 55 fixtures `index.md` de la suite declaran ahora
    `type`/`title`/`description`; es deuda transitoria que E16-H05 barre.

- ✅ **E16-H03** — **Título derivado**. `model::derived_title(fm, body, path)`: `frontmatter.title`
  (si es escalar y no vacío) → primer **H1** del cuerpo → nombre del fichero sin `.md`. Función pura
  y **total** (`String`, no `Option`). Muere `model::title_from_path` y con ella el Title Case con el
  quirk del `\b` de JS (`año.md` → `AñO`), junto al test de paridad
  `title_from_path_boundaries_como_js` — el prototipo dejó de arbitrar en E15-H04. `model::Heading`
  gana el campo `level` para poder distinguir el H1 del primer heading. Un `title` no escalar (lista,
  mapa, `null`) o vacío cae al siguiente eslabón, y **nunca** se reescribe el dato del usuario.
- ✅ **E16-H02/H03** — **Ningún fichero reservado + título derivado**. Fuera `FileKind`,
  `file_kind`, `is_reserved`, `RelPath::is_reserved`/`concept_id`, la rama de reservados de
  `parse_file`, `root_okf_version`, el gating «reservado antes de negar» de la query y el quirk de
  `graph.rs`. `Analysis` pierde `in_index`/`okf_version`; `Backlinks` pierde `index_refs`;
  `orphans` → `isolated` **con definición nueva** (sin entrantes NI salientes) y deja de ser
  diagnóstico. `derived_title` = `title` escalar → primer H1 → nombre del fichero; muere
  `title_from_path` con su Title Case heredado del quirk `\b` de JS. Radio hasta el store
  (`files.kind` y `links.src_is_index` fuera, `USER_VERSION` 2→3). Contrato: `graph_query`
  `"orphans"` → `"isolated"` **sin alias** (la semántica cambió: un alias devolvería otra cosa bajo
  el mismo nombre). 271 tests.
- ✅ **E16-H04/H05** — **Patch quirúrgico + catálogo mínimo**. `patch_frontmatter` edita el **texto
  crudo** del bloque línea a línea con un splice del `span`, sin round-trip por `serde_yaml` en el
  camino feliz; un comentario YAML del usuario sobrevive (es el testigo de que no hubo round-trip).
  Verificación cruzada contra el `Mapping` parseado para descartar claves duplicadas, anchors y
  alias. **Cierra un riesgo de pérdida de datos que no tenía criterio**: la frontera no es «no tengo
  mapa que parchear» sino «hay un bloque del usuario que no sé leer y no voy a pisar» — decidido
  sobre `split_front`, no sobre `frontmatter.is_none()`, que confunde ausencia con ilegibilidad.
  Llega a producción vía `merge_frontmatter` y `plan::apply_one`. `CheckCode` pasa al catálogo de
  `§20.9`: ni un `OKF-*`. 282 tests.
- ✅ **E16-H06** — **`Concept` → `Document`** (cierra E16). 54 ficheros; `core::bundle` →
  `core::document_set`; wire `CONCEPT_NOT_FOUND` → `DOCUMENT_NOT_FOUND` sin alias. **E16 COMPLETA.**
  283 tests.
- ⚠️ **Deuda declarada al cerrar E16**: (1) `Severity::Warn` se ha quedado **sin productor** en
  `all_checks`, así que `PlanPolicy::allowWarnings` y `gate.blockWarnings` son inalcanzables desde
  datos reales hasta que E17 traiga `LINK-CASE-MISMATCH` y E20 la política de severidades; (2) la
  pareja `Conformant → Valid` de `§20.3` está a medias — ver `decisiones §13` (cerrada
  después en E23-H14);
  (3) `core::types` sigue documentando el `.d.ts` generado por ts-rs, falso desde que se retiró la UI.

### E17 — Enlaces y grafo universal

- ✅ **E17-H01/H02** — **Extracción y resolución de enlaces**. Entra `pulldown-cmark` en el core
  (puro: arrastra solo `bitflags`/`memchr`/`unicase`). El `href` se deriva del **span**, no del
  `dest_url` del parser, así que `body[span] == href` es cierto por construcción y el destino llega
  crudo — lo que necesita `move_document` para reescribir el byte exacto. En un enlace de
  **referencia** el span cae dentro de la **definición**.
  - **Hallazgo que evita inventar enlaces**: se activan `ENABLE_TASKLISTS` y `ENABLE_FOOTNOTES`
    porque sin ellas el `[x]` de `- [x] hecho` es sintácticamente un enlace corto, y con un `[x]: …`
    en el documento se convierte en una arista del grafo que nadie escribió.
  - **Bug real corregido**: la contención cuenta **profundidad**, no recorta. `model::normalize`
    colapsaba `..` con `pop()` sobre vector vacío (no-op silencioso), así que `docs/auth.md` +
    `../../docs/auth.md` —que sale del workspace y vuelve a entrar— resolvía a un `Document`
    válido. El percent-decoding va **después** de interpretar `.`/`..` (RFC 3986).
- ✅ **E17-H03/H04/H05** — **Diagnósticos, `Analysis` nueva y superficie**. Muere el parser
  heredado (`LINK_RE`, `resolve_link`, `out_links*`, `raw_rel_links`) con sus 12+ consumidores
  migrados. `Analysis` pasa a los seis campos de `§20.7`, con `hard_fail`/`warn_count` como
  **métodos derivados** (un contador que no puede desincronizarse de su lista). `LinkReference` y
  `DanglingLink` **anidan** el `ResolvedLink`, así que `incoming` es literalmente la inversa de
  `outgoing`. El store materializa las **aristas** pero **sintetiza** los diagnósticos de enlace,
  porque dependen del inventario entero (crear un fichero repara el enlace de otro).
  **315 tests · E17 COMPLETA.**
  - **Cambio de comportamiento**: un enlace a un documento inexistente es ahora `Err`, así que
    `create_document` con la política por defecto **rechaza** crear un documento con un enlace
    «hacia el futuro» (consecuencia de `danglingDocumentLinks: error`, `§20.9`).
  - **Coste conocido**: un enlace a la **raíz** del workspace da `LINK-ESCAPES-WORKSPACE`, porque un
    destino que normaliza a la raíz no es nombrable como `RelPath`. El arreglo correcto es ampliar
    `LinkTarget`, no parchear el diagnóstico → E20/E21.
  - **Deuda de test**: la guarda de `diagnosticos.rs:208` nombra `LinkStub`/`LinkRel`, las variantes
    que la historia manda borrar. Se conservan **declaradas y sin productor**; retirarlas es una
    línea cuando se retire esa guarda.
- ✅ **Cableado de `other_files`** (cierre de E17). `DocumentSet::with_other_files` no tenía **ni un
  llamador**: `Inventory::contains_file` devolvía siempre `false` y la rama `WorkspaceFile` era
  código muerto en ejecución, así que **todo** enlace a código salía `Missing` — y sobre un destino
  `.md` excluido eso es `Err`, o sea que **tumbaba la puerta de CI por un fichero que estaba ahí**.
  `Discovered` gana `other_files` (todo lo que el walker **visita** y no acaba en el inventario;
  cero I/O extra, medido en 4,4 ms sobre este repo con 48 documentos y 98 `other_files`).
  - **Bug destapado y corregido**: los fantasmas del grafo se acotan a documentos Markdown. Todo
    `Missing` se convertía en nodo sin mirar si el destino era siquiera un `.md`, contra `§20.7`.
    Estaba oculto porque, mientras todo enlace a código era `Missing`, los ficheros de código eran
    **siempre** fantasmas y ningún test lo miraba (`codigo_no_es_nodo` solo cubría el que sí existe).
    El filtro por extensión se aplica **solo a `Missing`**, nunca a `Document`: un `Document` está en
    el inventario y lo es aunque `discovery.include` admita otra extensión — filtrarlo por el nombre
    sería la clasificación por extensión que `§20.6` prohíbe.
  - **Verificado end-to-end**: las 5 clasificaciones de `§20.6` sobre un repo real, con el enlace de
    **referencia** resuelto por su definición y **un solo** diagnóstico (el enlace realmente roto),
    donde antes había dos. **317 tests.**
- ⚠️ **Asimetrías declaradas al cerrar E17**: (1) el camino transaccional (`change_plan` y el gate de
  staging) construye el `DocumentSet` **sin** `other_files`, así que un plan sobre un documento con
  enlaces a código verá en `diagnosticsAfter` un `LINK-TARGET-MISSING` que el `before` ya no tiene;
  (2) la cache resuelve con `Inventory::default()`, que solo coincide con el core mientras los
  documentos sean `.md`; (3) los **diagnósticos de descubrimiento se siguen descartando** — con
  dueño en E20, ver `requirements/README.md`.

### E18 — Store v2

- ✅ **E18-H01/H02** — **DDL v2**. `files` → `documents(path, title, body, raw, frontmatter_json,
  content_hash)` sin las columnas OKF promovidas; `metadata(document_path, field_path, value_json,
  value_type)` poblada con `ParsedFrontmatter::walk` —el reflejo exacto de `get`, así que
  `get(path)==Some(value)` por construcción y no hay un segundo navegador del `Value`—; `links` gana
  `target_kind`/`fragment`/`resolved`/**`is_edge`** (este último computado por el core: hace exactas
  las consultas de grafo incluso bajo `upsert` incremental, porque no depende del inventario vivo);
  `diagnostics` gana `range_json`. El store **materializa `other_files`** para clasificar los enlaces
  a código, cerrando la asimetría de `Inventory::default()`. `walk` es la firma que heredan E19/E20.
- ✅ **E18-H03/H04** — **FTS genérico + paridad completa** (cierra E18). `documents_fts(path, title,
  body, frontmatter_text)` sin `description` privilegiado, alimentado en el mismo recorrido `walk`.
  La paridad core↔store vuelve a comparar la `Analysis` **completa** del modelo nuevo, incluida la
  clasificación de enlaces; la fase roja **verificó empíricamente** que la síntesis de diagnósticos
  aún reconstruía el `DocumentSet` sin `other_files` (enlace a código: `warn_count` store=2 vs
  core=1), y H04 lo cerró propagándolos a `synth::link_diagnostics` y al trait `DocumentStore`.
  **329 tests · E18 COMPLETA.** El store se reconstruye sin un solo dato OKF.

### E19 — Lenguaje de consulta genérico

- ✅ **E19-H01** — **Evaluador tipado**. `Expression`/`ComparisonOperator`/`QueryValue`/`ValueType`/
  `TypeError` en `core::types`; `eval::evaluate`. La asimetría rectora: orden cruzado (número vs
  string) es `TypeError`, igualdad cruzada es `false`. Va **siempre** sobre `get`, nunca `get_text`.
  `TypeError` es tipo propio (no variante de `CoreError`): un `where` mal tipado es entrada del
  agente, no un fallo del núcleo.
- ✅ **E19-H02/H04** — **Parser textual + namespaces**. `parse` es descenso recursivo a mano, cero
  deps. La abreviatura normaliza a la forma **desnuda** (`frontmatter.status` → `["status"]`).
  `document.*`/`graph.*` **sintetizan** un `Value` de su tipo natural y lo pasan por la **misma**
  maquinaria de tipos de H01, así que `graph.backlinks >= "x"` es un `TypeError` gratis.
- ✅ **E19-H03** — **Filtro JSON + equivalencia**. `filter::from_json` con un tipo wire intermedio;
  `value`/`operator` deserializan solos por los atributos serde de H01. La equivalencia con el `where`
  textual es **exacta** porque comparten `build_field_path` — mismo AST, comparado estructuralmente.
- ✅ **E19-H05** — **Cableado a `knowledge_search`** (cierra E19). `where`/`filter` → `Expression`,
  intersectados con el FTS de `text`. `SearchResult` pierde los campos OKF. La DSL de subcadena
  (`query.rs`) se borra entera, pero `loose_text_match` se **reubica** en `text.rs` porque el store
  lo invoca. Un `TypeError` por-documento **excluye** ese documento sin abortar la búsqueda.
  **362 tests · E19 COMPLETA.**
  - **Verificado end-to-end** sobre un proyecto real: `where "status = \"accepted\" and priority >= 2"`
    y el `filter` JSON equivalente dan el **mismo** resultado; `owners contains "security"` filtra por
    un valor de lista; `priority >= "high"` excluye los documentos con `priority` numérico (la regla
    de tipos, viva a través de MCP).

### E20 — Inspección de metadata y validación genérica

- ✅ **E20-H01/H02** — **`metadata_inspect` (catálogo + campo)**. Funciones puras del core sobre
  `ParsedFrontmatter::walk` (cuarto consumidor del mismo iterador: store, evaluador, namespaces,
  catálogo — ninguno puede discrepar de qué es un campo). El catálogo incluye los mapas intermedios
  (son direccionables); `values` cuenta solo escalares, orden determinista con desempate final por
  `ValueType`. 368 tests.
- ✅ **E20-H03** — **`metadata_inspect` sustituye a `schema_inspect`; `core::schema` borrado**. Muere
  la última maquinaria de schema OKF: `DocType`/`RelationDef`/`validate_schema`/`.lodestar/schema.yaml`
  y las variantes `SCHEMA-*`/`REL-*`/`EXTREF-MISSING`/`LINK-STUB`/`LINK-REL` de `CheckCode`.
  **`referenceRoots` conservado** (sostiene la write policy, no era OKF); el gate de CI recompuesto
  con `LINK-TARGET-MISSING`. Verificado e2e: `metadata_inspect` descubre el catálogo, `schema_inspect`
  da «tool desconocida». 351 tests.
- ✅ **E20-H04** — **Política de validación + diagnósticos de descubrimiento** (cierra E20). Salda la
  deuda de E15-H07: los diagnósticos de descubrimiento (`DOC-NOT-UTF8`, `LINK-CASE-MISMATCH`…) llegan
  al reporte de `knowledge_check`, por un canal aparte porque su target no es un documento. La
  severidad por familia de `validation` se aplica (`ignore` suprime). El **gate diferencial**
  `rejectNewErrors`/`allowExistingErrors`: un apply sobre un repo que ya tiene errores se permite si
  no introduce otros nuevos —la comparación antes/después que hace a Lodestar usable sobre un proyecto
  real—. **356 tests · E20 COMPLETA.**

### E21 — Contrato MCP y transacciones genéricas

- ✅ **E21-H01** — **Retiradas las 5 operaciones semánticas**. `NormalizedOperation` queda en las 8
  universales; `impact_analyze.kind` restringido a `{move, delete}`. Sin pérdida de capacidad: un
  `transition_status` es un `patch_frontmatter` (probado por test). La mecánica transaccional intacta.
- ✅ **E21-H02/H03** — **Selecciones masivas + move/delete genéricos**. `change_plan` acepta
  `{selection: {where|filter}, operation}` → una op por documento que casa; `capturedRevisions` con la
  revisión de cada uno. `move_document` reescribe backlinks por el **`span`** (cubriendo las
  definiciones de referencia que la regex no veía; spans procesados de mayor a menor offset).
  `delete_document` **exige política explícita** (`§Fase 12`: no elegir en silencio).
- ✅ **E21-H04** — **`OkfDiff` → `SnapshotDiff` + limpieza del contrato** (cierra E21). El diff de wire
  ya era `types::SemanticDiff` (E12); el de `diff.rs` pasa a `SnapshotDiff` (neutro). Contrato sin
  vocabulario OKF en superficie activa; `decisiones §13` (`Conformant → Valid`) documentada como
  aplazada (toca el catálogo de errores congelado).

### E22 — Migración de repos OKF y publicación

- ✅ **E22-H01** — **`migrate-from-okf --dry-run`**. Diagnóstico de cortesía que detecta las
  convenciones OKF legadas (`index.md` raíz, índices anidados, `okf_version`, índices de tags) **sin
  modificar ningún fichero** (modo hermético `open_ephemeral`; verificado byte a byte). No es puerta:
  exit 0 siempre que pueda leer.
- ✅ **E22-H02/H03** — **README reescrito + v0.3.0 incompatible**. README con la definición de `§20.1`,
  las 10 tools (verificadas 1:1), sin OKF/UI/git. Bump `0.2.0 → 0.3.0` y entrada de `CHANGELOG` con el
  aviso de incompatibilidad.
- ✅ **E22-H04** — **Verificación e2e de la migración completa**. `flujo_completo_migracion`
  (`crates/lodestar-mcp/tests/e2e_migracion.rs`) recorre el flujo del `§Resultado esperado` por la
  **superficie MCP JSON-RPC real** sobre un proyecto arbitrario sin `.lodestar/`/`index.md`/frontmatter:
  descubrimiento → `workspace_status` → `knowledge_search` con `where` tipado (incl. la regla de tipos)
  → equivalencia `where`/`filter` → `knowledge_get` con enlaces clasificados → `metadata_inspect` →
  `graph_query` (backlinks globales, aislados) → **selección masiva** `change_plan` → `change_apply` →
  `knowledge_check` → `change_revert`. **Todo verde.**
  - **Bug de cableado que el e2e destapó (el 5º de la sesión con ese patrón)**: la **selección masiva
    no llegaba a la superficie MCP**. `App::change_plan` sabía interpretar `{selection, operation}`
    (E21-H02, probado por unit-test directo sobre `App`), pero el dispatch de `tools::call` extraía
    solo `params["operations"]` (el array) y el `inputSchema` tenía `additionalProperties: false` +
    `required: ["operations"]`, así que descartaba `selection`/`operation`. Corregidos dispatch y
    schema. **Ningún test unitario lo cazó porque probaban `App`, no la frontera.**

### E23 — Cierre de la migración

> Épica de **cierre**, abierta por la revisión de la PR #17 (2026-07-25). No es una fase de `§20.14`:
> salda los defectos que E15–E22 dejaron vivos antes de mergear. Su bloque A **no se dedujo leyendo
> código: se reprodujo ejecutando los binarios**, y esa es la lección de la épica — de los defectos
> que aparecieron, **cinco no estaban en la revisión inicial** y salieron implementando.

- ✅ **E23-H06** — **El CI corre los tests de crash-recovery**. Los 4 de E13-H06 estaban tras
  `#[cfg(feature = "test-failpoints")]` y `cargo test --workspace` **no activa features opcionales**:
  llevaban sin ejecutarse desde E13. La garantía nuclear del motor (un crash nunca deja un `.md` a
  medias) era verde por no correr. Ahora el CI los lanza en un step propio.
- ✅ **E23-H01/H04** — **Una sola verdad de validación y `pendingTransaction` real**. `lodestar check`
  y `knowledge_check` daban **veredictos contradictorios sobre el mismo workspace**: `full_analysis`
  ignoraba la sección `validation` de la config y los diagnósticos de descubrimiento. Y
  `recovery.pendingTransaction` era un `false` literal desde E10-H08 pese a que la detección existía
  desde E13-H06: tras un crash, la primera tool que llamaba un agente le mentía.
- ✅ **E23-H02/H03/H05** — **El camino de escritura deja de mentir** (cierra el bloque A). `create`
  escribía `type: ''` (residuo OKF) y un `title` que nadie pidió; **no se podía mover una nota que
  enlazara a sus vecinas** porque los salientes del documento movido no se recalculaban y el gate lo
  veía como errores nuevos; y `delete` aceptaba `retarget`/`create_stub` **sin ejecutarlas**,
  dejando los entrantes rotos — retiradas, porque ninguna tiene semántica que inventar sin más datos
  del llamador.
- ✅ **E23-H07/H08** — **e2e de ciclo de vida en UNA sola sesión MCP**, y `reindex` deja de no tener
  tests. Antes cada paso del e2e levantaba un proceso, lo que **enmascaraba justo los bugs de
  invalidación**; el ignorado (`.gitignore`/`.lodestarignore`) tampoco se probaba por ninguna fachada.
- ✅ **E23-H09/H11 (core)** — **Bordes del motor y descubribilidad**. Concurrencia entre **procesos**,
  lock huérfano, unicode en rutas, `patch_frontmatter` sobre YAML ilegible, `--root` inexistente. Y
  en el core: `metadata_inspect` no daba el **vocabulario de tags** (no explotaba las listas, el caso
  de uso número uno de una KB de notas), y `[volver](../)` tumbaba la puerta de CI.
- ✅ **E23-H10** — **El schema de escritura deja de ser opaco**. El `inputSchema` de `change_plan`
  declaraba 4 campos; ahora declara los **18** que el código lee. Para un producto cuyo público son
  agentes, ese era el mayor agujero de usabilidad: el schema decía qué ops existen pero no cómo
  invocarlas.
- ✅ **E23-H23/H24** (bloque F, no planificado: los destapó el propio bloque B) — **Lock reclamable
  por TTL+PID** (un proceso muerto por SIGKILL dejaba la base cerrada a la escritura **para siempre**)
  y **NFC/NFD** (un enlace correcto tumbaba el CI en macOS, con el mismo veredicto que en Linux pero
  acertado solo en una de las dos plataformas). Resueltos como decidió el usuario: reclamo por
  TTL+PID, y resolución tolerante con aviso sin normalizar la ruta canónica.
- ✅ **E23-H12** — **Abrir un workspace deja de escribir en el proyecto ajeno**. `Workspace::open`
  llamaba a `ensure_gitignore` y `ensure_runtime_scaffold`, así que `lodestar check` y arrancar el
  MCP —**incluso en perfil `readonly`**— reescribían el `.gitignore` del usuario y creaban
  `.lodestar/runtime/` antes de leer nada. Los dos efectos pasan a ser **perezosos**: el scaffold se
  retira sin sustituto (cada consumidor crea su directorio antes de escribir) y el ajuste del
  `.gitignore` vive en `Workspace::ensure_managed_gitignore`, invocado desde los cuatro chokepoints
  que cubren todo camino de escritura (`enable_cache`, `acquire_lock`, `persist_plan`,
  `try_append_audit`). Con los efectos fuera, `open_ephemeral` quedaba idéntico a `open` y **se
  retira: abrir ya es hermético**. Además se retiran `implemented_by`/`verified_by` como claves de
  frontmatter privilegiadas —último residuo OKF con semántica impuesta, contra el invariante 3 de
  `§20.2`— y con ellas la opción `include:["externalReferences"]` de `knowledge_get`. Cierra de paso
  una grieta del invariante #4: `ExternalReference` era un tipo de wire definido **fuera** de
  `core::types`.
  - **Tres defectos que destapó el ciclo, ninguno en la revisión inicial**: `receipt_gc` dependía
    del scaffold para plantar sus ficheros, y al arreglarlo salió que escribía su `config.yaml`
    **después** de abrir el workspace —y la config se lee una sola vez al abrir—, así que el GC
    llevaba **desde E13-H07 corriendo con los defaults**; como los defaults coincidían con lo
    declarado, era indistinguible de un `gc_receipts` que ignorase la config entera, y el TTL
    (`retainReceiptsFor`) **no lo cubría ningún test**. La cobertura de dos de los cuatro chokepoints
    era **vacua** (el juez ciego lo demostró por mutación: borrando ambas llamadas la suite seguía
    verde) porque el test comprobaba primero tras `change_plan`, que ya ajusta el `.gitignore` — y su
    rustdoc afirmaba lo contrario. Y `materialize_staging` es API pública que escribe bajo
    `.lodestar/` sin pasar por ningún chokepoint (hoy sin llamador de producción; registrada).
  - **La superficie de ataque se cerró, no se trasladó**: el vector del test de path-traversal
    existía porque `Workspace::external_refs` era el único punto que convertía una cadena cruda de
    frontmatter en un `is_file()`. Ese código desaparece; la propiedad se migra endurecida a
    `frontmatter_no_es_oraculo_de_ficheros_del_host`, que pasa de prohibir `exists:true` a prohibir
    **cualquier** resolución.
- ✅ **E23-H11 (resto)** — **La KB se vuelve descubrible y la superficie deja de prometer de más**.
  `knowledge_search` acepta `include: ["frontmatter.<fieldPath>"]`: el sufijo se parsea con
  `FieldPath::parse` y se resuelve con `ParsedFrontmatter::get`, así que los anidados salen gratis.
  Antes ver el `status` de 30 resultados costaba **30 `knowledge_get`**. Valores YAML crudos sin
  coerción; un campo ausente no aparece (distinto de un `null` explícito, con test que fija la
  distinción); proyecta **solo lo pedido**. `sort` **se retira** en vez de implementarse (se aceptaba
  y se ignoraba en silencio; volver a añadirlo es aditivo). `apply_fix` **se retira**: sin productor
  de `Fix` desde E20-H03 fallaba siempre, y devolvía `DOCUMENT_NOT_FOUND`, mandando al agente a
  buscar el problema donde no estaba — el lado de **lectura** (`Fix`, `Check.fixes`,
  `includeSuggestedFixes`) se conserva, porque un array vacío no engaña. Y los **receipts se listan
  desde `workspace_status`**, no como 11ª tool: perder el `receiptId` dejaba el undo inalcanzable
  pese a estar persistido.
- ✅ **E23-H13/H14/H15/H16** — **Los documentos dejan de contradecirse**. La tabla de la migración
  decía «E15–E22 EN CURSO» **350 líneas por encima del detalle que las daba por cerradas** (era
  criterio de aceptación literal de E22-H03, incumplido); la cabecera describía la UI Tauri y
  subcomandos git borrados en E9-H02. `decisiones §12` (fechas **lexicográficas**, porque
  `serde_yaml` 0.9 no tipa timestamps) y **§13** (`Conformant → Valid`) cerradas: §13 era el **único
  de los 29 criterios de `REFACTOR_PHASE_2` demostrablemente incumplido**, y se saldó abriendo el
  catálogo de 16 códigos **la única vez**, aprovechando que v0.3 ya era incompatible con v0.2. Y se
  escribieron `docs/history/PROPUESTA_CLI.md` y `decisiones §14` (el store de E18 **entero, sin ningún
  consumidor**) para que dos decisiones no se perdieran.
  - **Lo que la barrida final encontró y que nadie había mirado**: el texto `instructions` que el
    servidor sirve en `initialize` —lo primero que lee un agente, y **superficie de wire**, no
    documentación— usaba vocabulario retirado (`huérfanos` cuando la operación es `isolated` desde
    E16-H02, `conforme` cuando E23-H14 lo pasó a `valid`). Un agente que leyera las instrucciones y
    probase `orphans` se comía un `INVALID_SCHEMA`.
- **437 tests · E23 COMPLETA.** (+4 de crash-recovery tras `--features test-failpoints`.)

## Cierre de defectos de la v0.3.0 (E24) — v0.3.1 + v0.4.0 · COMPLETA

> Rama `release/v0.3.1`. Épica: [`epica-24`](requirements/epica-24-cierre-defectos-v031.md).
> **Origen**: revisión de la v0.3.0 publicada (2026-07-28). Igual que en E23, ningún defecto se
> dedujo leyendo código: los cinco se reprodujeron **ejecutando** `lodestar-mcp` por JSON-RPC sobre
> stdio y la CLI contra workspaces de prueba, con un arnés de sesión viva.
>
> La v0.3.0 **pasa todas sus puertas** —437 tests, `clippy -D warnings`, los 4 de crash-recovery,
> pureza del core— y el invariante nuclear aguantó **30 `SIGKILL` reales** durante `change_apply`
> sin dejar ni un `.md` a medias. Estos son defectos que la suite **no mira**.

**Corte de release**: `H07`/`H08` (lenguaje de consulta) quedan **fuera de v0.3.1** y son el núcleo
de **v0.4.0**: cambian resultados observables de consultas que hoy se aceptan y una revisa un
criterio ratificado en E19-H04. Ninguna historia de v0.3.1 depende de ellas.

| Historia | Estado | Detalle |
|---|---|---|
| **E24-H01/H02** El BOM deja de tragarse el frontmatter, y se hace visible | ✅ | Pérdida silenciosa de datos. `DOC-BOM` (aviso, no configurable). |
| **E24-H03/H04** La recuperación tras un crash deja de estorbar | ✅ | Se acabó el `WRITE_CONFLICT` sistemático; `check` avisa en vez de mentir. |
| **E24-H05/H06** El plano de control deja de crecer sin cota | ✅ | `StagingDir` RAII; el GC barre huérfanos y corre también al fallar. |
| **E24-H09/H10** La superficie de error deja de mentir | ✅ | Valores validados; 0 de 21 errores sin código (antes 10). |
| **E24-H11** `knowledge_get` devuelve `title` | ✅ | Faltaba justo en la tool que lee un documento. |
| **E24-H12** Huecos de descubrimiento | ✅ | `README.MD`, symlink de directorio, `a\b.md`. |
| **E24-H13/H14** El crash se prueba de verdad | ✅ | Seam real en el orquestador + `SIGKILL` al binario. |
| **E24-H15/H16/H17** La suite muerde donde no mordía | ✅ | `outputSchema` en las 10; escala por el wire; los tests que E23-H10 prometió. |
| **E24-H18** Documentos y publicación | ✅ | v0.3.1. |
| **E24-H07/H08** Lenguaje de consulta | ✅ | **v0.4.0**, tras publicar v0.3.1: el typo bajo namespace reservado falla, y `frontmatter.` vuelve a ser un anclaje. |

- ✅ **E24-H01** — **Pérdida silenciosa de datos por BOM UTF-8**. `split_front` comparaba
  `raw.starts_with("---")` sobre un raw que empieza por `\u{feff}---`, así que un `.md` con BOM caía
  en `SplitFront::Sin`: su metadata era **invisible** para el motor y `knowledge_check` respondía
  VÁLIDO con 0 diagnósticos. Al escribir encima, la rama `Sin` de `patch_frontmatter` anteponía un
  bloque nuevo **por delante** del BOM (dos bloques, el original degradado a cuerpo) y un
  `replace_body` posterior destruía la metadata original para siempre.
  El arreglo cubre **los cinco caminos de reescritura de cuerpo de una vez**: `replace_text` y
  `edit_section` normalizan a `ReplaceBody`, igual que `move --rewriteInboundLinks` y
  `delete remove_links`. El BOM se **conserva byte a byte** y NO se normaliza al leer de disco:
  `workspace_revision` hashea los bytes crudos.
  - **Colateral encontrado implementando**: `links::diagnose` asumía `body_start == 0` sin bloque,
    así que los rangos de los diagnósticos de enlace de un `.md` con BOM y sin frontmatter iban
    desplazados 3 bytes. Ahora consume `SplitFront::body_offset`.
  - **Lo que el juez ciego encontró y la fase roja no**: tres ramas del arreglo **sobrevivían a la
    mutación con la suite entera en verde**. Dos eran mayores y se cerraron con tests verificados
    poniendo cada mutación (`move_no_se_come_el_bom_del_enlazante_sin_frontmatter`,
    `patch_sobre_bom_sin_bloque_no_precede_a_la_marca`). Es la lección de E23 repitiéndose: los seis
    tests de la fase roja pasaban con el defecto reintroducido.
- ✅ **E24-H02** — **El BOM deja de ser invisible**. Nuevo `CheckCode` `DOC-BOM` (`warning`,
  severidad **intrínseca**: `family_of` devuelve `None`, como `LINK-ESCAPES-WORKSPACE`). Se emite
  antes del corte por frontmatter ilegible, porque la marca es del **fichero**, no del bloque.
  - **Aviso de release**: hasta v0.3.0, un BOM ocultaba el frontmatter entero, así que un workspace
    con un `.md` con BOM y frontmatter ilegible pasaba `lodestar check` con **exit 0**. Desde v0.3.1
    ese documento se interpreta y sus problemas reales se diagnostican: **el veredicto de la puerta
    de CI puede cambiar a exit 1** sobre bases existentes. Es el comportamiento correcto, pero es un
    cambio observable y va en la nota de release.

- ✅ **E24-H03/H04** — **La recuperación tras un crash deja de estorbar**. `change_plan` leía el
  disco **sin recuperar** y fijaba ahí su base; después `apply_transaction` recuperaba por debajo y
  el control optimista lo veía como conflicto ajeno → **`WRITE_CONFLICT` en la primera escritura,
  siempre** (10 de 11 reproducciones matando el servidor con `SIGKILL`). El código además mentía: lo
  había alterado la recuperación del propio Lodestar. Ahora el plan recupera —bajo el mismo lock de
  publicación, y solo si hay algo que recuperar— antes de leer. Y `check` **avisa** de la
  transacción pendiente en vez de informar de 120 enlaces rotos como si fueran daño real; avisa, no
  repara (abrir sigue siendo hermético, E23-H12).
  - **Decisión, en contra de lo que pedía la spec**: NO se le fabrica un emisor a
    `WORKSPACE_RECOVERY_REQUIRED`. Con la recuperación transparente, ese código pasa a ser
    inalcanzable **por diseño**, no por defecto: el agente ya no puede encontrarse un workspace que
    exija recuperación manual. Inventarle un emisor sería forzar la superficie para cumplir una
    spec escrita antes de decidir H03.
- ✅ **E24-H05/H06** — **El plano de control deja de crecer sin cota**. `StagingDir` no era RAII y
  los pasos (7)–(10) de `apply_transaction` salen por `?`: toda transacción fallida dejaba el árbol
  `.md` **completo** de su resultado. Y `gc_receipts` iteraba solo `receipts/` y solo corría en el
  camino de éxito — el flujo que producía la basura era el que no la recogía.
- ✅ **E24-H09/H10** — **La superficie de error deja de mentir**. Los valores de los parámetros
  declarados se validan de verdad (era la política que el contrato ya prometía: `limit: 0` devolvía
  0 resultados en silencio pese al `minimum: 1` declarado), y **0 de 21** errores viajan sin código
  del catálogo (antes 10). La misma consulta malformada daba **dos códigos distintos** según la
  tool. Nueva variante `WorkspaceError::InvalidSchema` para no seguir aplanando entrada inválida
  contra fallo de I/O. Lo que **no** cambia: los parámetros no declarados se siguen ignorando — eso
  es revisar un criterio ratificado, y queda como `decisiones §15`.
- ✅ **E24-H13/H14** — **El crash se prueba de verdad**. La feature `test-failpoints` existía desde
  E13-H06 pero **ningún fichero de `src/` la referenciaba**: los tests componían el estado
  post-crash a mano y **en orden distinto al del orquestador** (journal antes que backup, cuando
  producción hace backup antes que journal), así que `TrasJournalPrepared` describía un estado que
  el código real no puede producir y pasaba vacuamente. Ahora el punto de caída se inyecta dentro de
  `apply_transaction`, y un test aparte mata el **binario** con `SIGKILL`.
- ✅ **E24-H15/H16/H17** — **La suite muerde donde no mordía**. El `structuredContent` se valida
  contra el `outputSchema` en las **10** tools (antes 5, y solo que el schema «tuviera alguna clave
  estructural»); la escala se mide **por el wire** (10.000 documentos → ~73 KB); y se escriben las
  afirmaciones que el repo daba por ciertas: los dos tests que E23-H10 declaró como criterio y
  nunca existieron, y el grep de CI de los `ErrorCode` — que **al ejecutarlo cazó una violación
  real**: `WorkspaceError::code()` mantenía su propia tabla de códigos de wire.
- **486 tests · E24 COMPLETA** (+ los de crash-recovery tras `--features test-failpoints`).
- ✅ **E24-H07/H08 (v0.4.0)** — **El lenguaje de consulta deja de responder a lo que no entiende**.
  `graph.backlink = 0` (con typo) devolvía `[]`: indistinguible de un resultado legítimamente vacío,
  o sea una respuesta **silenciosamente equivocada**, que es peor que un error. Ahora falla con
  `INVALID_SCHEMA` nombrando las propiedades válidas. Y `frontmatter.` vuelve a ser un **anclaje**:
  se descartaba como abreviatura, así que un frontmatter de usuario con una clave llamada `graph` o
  `document` era **inalcanzable por cualquier consulta** pese a que `metadata_inspect` lo anuncia —
  justo el flujo que las `instructions` del servidor recomiendan.
  - Se valida en `parse::build_field_path`, el **único** punto compartido por `where`, `filter` y
    `has`/`missing`, así que la equivalencia entre los tres se preserva por construcción.
  - Las propiedades válidas pasan a `core::types` (`DOCUMENT_PROPS`/`GRAPH_PROPS`): vivían solo en
    los brazos de un `match` de `eval.rs`, donde validador y evaluador podían divergir.
  - **Revisa el criterio de E19-H04** («una sub-clave de namespace desconocida es propiedad
    ausente»), que era deliberado. Por eso fue a v0.4.0 y no al parche.

## Endurecimiento de la escritura y de la superficie de errores (E25–E26) — COMPLETAS

> Rama `epic/e25-e26-endurecimiento` (de `e183d00` a `7ebe764`). Épicas:
> [`epica-25`](requirements/epica-25-endurecimiento-escritura.md) ·
> [`epica-26`](requirements/epica-26-ux-errores.md). **Origen**: auditoría del camino de escritura y
> de la superficie de errores (2026-07-29), posterior a v0.3.1 y al bloque C de E24.
>
> **Qué las distingue de E23/E24**: aquellas reprodujeron sus defectos **ejecutando** los binarios;
> estos once se localizaron **leyendo** el orquestador y la frontera, y todos comparten la razón por
> la que la suite no los veía: necesitan **dos actores** —dos procesos, o un proceso y una caída— y
> la suite ejercía uno. Cada historia abrió con una fase roja que monta el segundo actor.
>
> **Principios rectores**: en E25, *una salvaguarda vale por el estado sobre el que se ejerce, no por
> el estado sobre el que se computó*; en E26, el de E24-H07: *una respuesta silenciosamente
> equivocada es peor que un error*.

| Historia | Estado | Detalle |
|---|---|---|
| **E25-H01** La publicación no escribe fuera de lo que respaldó | ✅ | TOCTOU `[T1,T3)`: se acabó el borrado sin copia ni journal. |
| **E25-H02** Copias durables y verificadas; cuarentena en vez de encalle | ✅ | + el sellado del aborto de ventana (enmienda de la propia épica). |
| **E25-H03** El GC no desarma a una transacción viva | ✅ | El criterio de «vivo» deja de ser «tiene journal o recibo». |
| **E25-H04** Publicar implica recibo | ✅ | Recibo pendiente escrito con el journal, promovido al sellar. |
| **E25-H05** Borrar es durable, y revertir re-verifica y deja recibo | ✅ | Cierra el espejo de S5 que el juez de H04 destapó. |
| **E25-H06** El lock tiene dueño demostrable | ✅ | + el `.gitignore` del usuario deja de perder sus CRLF. |
| **E26-H07** Todo error lleva código **y** mensaje | ✅ | 8 de 10 tools devolvían el código pelado. |
| **E26-H08** Un `TypeError` se reporta, no excluye en silencio | ✅ | La respuesta recortada era indistinguible de la correcta. |
| **E26-H09** Un solo dialecto de dot-paths | ✅ | `metadata_inspect` anunciaba campos que nadie podía consultar. |
| **E26-H10** Ninguna respuesta viaja sin cota | ✅ | `graph_query` servía el grafo entero; `metadata_inspect`, N valores. |
| **E26-H11** El contrato describe el servidor que hay | ✅ | `/contrato --check` limpio; cinco deltas aplicados. |

- ✅ **E25-H01** — **La publicación escribía fuera de lo que respaldó**. `apply_transaction` computa
  canónico, resultado y afectados en **T1**, y sobre ese conjunto ejerce `assert_writable`, el backup
  y el journal; pero `publish_result` **releía** el canónico en **T3** y **recomputaba** el conjunto
  contra el resultado de T1, escribiendo o borrando lo divergente sin ninguna de las tres
  salvaguardas. Tres consecuencias reproducidas: una edición externa en la ventana se pisaba con un
  backup que ya no correspondía; un `.md` **nuevo** creado en la ventana se **borraba** sin copia ni
  entrada de journal (irrecuperable, y el recibo ni lo mencionaba); y un fichero aparecido bajo un
  `referenceRoot` sufría lo mismo, sin que el control optimista pudiera verlo siquiera —
  `workspace_revision` excluye lo que queda fuera de `writableRoots`. Ahora `publish_result` recibe
  el canónico de T1, compara, y aborta con `WRITE_CONFLICT` **antes del primer rename**.
  - **Seam nuevo**: el `failpoint!` de E24-H13 solo sabía **abortar**; hizo falta un punto que
    **ejecute un gancho del test y continúe**, para poder inyectar el segundo actor dentro de la
    ventana. Es el que reusan después H02, H05 y H06.
- ✅ **E25-H02** — **Las copias de recuperación no eran durables, ni se verificaban, y una rota
  cerraba el workspace para siempre**. Se copiaban con `std::fs::copy` y el manifiesto `.absent` con
  `std::fs::write`, ninguno con volcado, mientras el journal **sí** se fsyncaba: tras un corte podía
  quedar un journal durable apuntando a una copia truncada, que la restauración escribía **verbatim**
  sobre el canónico. Y si la copia era ilegible, `recover()` propagaba `Err` en sus tres brazos con
  el journal aún en disco: `recovery_pending()` seguía en `true` y **toda** escritura futura moría en
  el paso (2). Ahora las copias van por el protocolo durable (extraído a `io::write_bytes_atomic` +
  `io::sync_dir`), se verifican por huella `blake3` contra un sidecar de manifiesto antes de
  restaurar, y un journal irrecuperable va a `journal/quarantine/<txnId>/` —nada se borra: es
  material forense— con `RECOVERY_FAILED`, que gana así su **primer emisor real** y sale de
  `codigos_sin_emisor`.
  - **Enmienda de la épica, nacida de implementar H01** (commit `2e0d6ea`, spec): el aborto de
    ventana dejaba en disco su journal `prepared` y las copias de T1 con **cero renames**, así que la
    siguiente operación recuperaba… **restaurando T1 encima de la edición externa que el aborto
    acababa de proteger** —y borrando, vía `.absent`, el fichero nuevo del usuario—. Sin esto, las
    tres garantías de H01 duraban hasta la siguiente operación. El camino de aborto **sella su propio
    journal** bajo el mismo lock (journal primero, árbol después, para que una caída a mitad deje un
    huérfano legítimo). Se rechazó por escrito la generalización tentadora —*«no restaurar un
    `prepared` con cero `applied`»*—: `mark_applied` persiste **después** de cada rename, así que ese
    estado también describe una caída entre el primer rename y su anotación.
  - **Promesa recalibrada, declarada**: «converge a uno de los dos bordes» pasa a estar condicionada
    a que las copias verifiquen. Con copias corruptas lo garantizado es: nada se escribe a partir de
    una copia que no verifica, el material se preserva, el fallo tiene código propio y el workspace
    vuelve a ser escribible.
- ✅ **E25-H03** — **El GC desarmaba a una transacción viva de otro proceso**. `gc_receipts` corría
  **fuera del lock** (lo dispara la fachada, ya soltado) y purgaba todo `staging/`/`recovery/` sin
  journal ni recibo. Entre `backup_originals` y `create_journal` —la ventana que el propio
  `FailPoint::TrasBackupSinJournal` modela— una transacción tiene copias y **no** tiene journal ni
  recibo: el GC del proceso B le borraba el árbol al proceso A, que publicaba sin copias; y si caía,
  `restore_from_recovery` devolvía `Ok(())` de inmediato al no encontrar directorio, **sellando un
  estado parcial en silencio**.
  - **Test que pasaba por la razón equivocada**: `caida_entre_backup_y_journal` (E24-H13) afirmaba
    que el GC **debe** purgar ese árbol. Sigue siendo cierto —pero ahora porque el dueño está
    **muerto**, no porque falte el journal. Es la tercera vez en cuatro épicas (E23, E24, E25) que
    aparece un test verde apoyado en la premisa equivocada.
- ✅ **E25-H04** — **Publicar podía no dejar recibo**. Después de `publish_result` el disco ya está
  cambiado, pero quedaban pasos que salían por `?`: el sellado, y en la fachada `write_receipt` y
  `gc_receipts`. Cualquiera convertía una transacción **publicada** en `Err` **sin recibo**, y no
  había salida: `change_revert` respondía `PLAN_EXPIRED` para siempre (el recibo no existe) y
  reaplicar el plan moría con `PLAN_STALE` (la base cambió). Con un `SIGKILL` pasaba igual, y eso
  descartaba el arreglo barato: degradar los fallos post-publicación a *warning* no cubre un crash.
  Ahora el **recibo pendiente** se escribe con el journal —sus dos revisiones ya se conocen antes del
  primer rename— en `receipts/pending/<txnId>.json`, y su **efectividad la decide el estado `applied`
  del journal** (`pending_receipt_efectivo`), de modo que su vida queda contenida en la del journal y
  no hace falta una tercera señal que caducar: el sellado lo promueve, la vía COMPLETAR de la
  recuperación también, y la vía RESTAURAR lo descarta.
  - **Colateral cerrado**: la promoción ocurre **bajo el lock**, lo que elimina de paso el hueco
    `[sellado, recibo)` en el que el GC —cuyo criterio de vivos es `journal/ ∪ receipts/`— podía ver
    la transacción como basura y purgarle las copias con las que se revierte.
- ✅ **E25-H05** — **Borrar no era durable, revertir no re-verificaba, y la reversión no dejaba
  recibo**. Tres cosas: `io::delete` hacía `remove_file` sin fsync del directorio (un corte tras el
  journal `applied` resucitaba el documento y el recibo mentía); el fsync de directorio era
  best-effort silencioso; y `change_revert` comparaba la revisión **antes** del lock, que lo toma
  `revert_transaction`, así que una edición externa en esa ventana se sobrescribía en silencio — el
  apply sí re-verifica bajo el lock desde E13-H02, el revert no tenía equivalente.
  - **MAYOR-2 del juez de H04, y por qué la spec se enmendó** (commit `cde2856`): la reversión
    conservaba **la forma exacta** del defecto que H04 acababa de cerrar. `write_receipt` de la
    inversa salía por `?` tras publicarla, y `revert_transaction` no persistía ningún registro
    durable antes de su punto de no retorno. Un `SIGKILL`/`ENOSPC` entre el último rename de la
    inversa y su recibo devolvía `Err` sobre algo publicado y sin recibo — y como el recibo es el
    criterio de «vivo» del GC, el árbol de la inversa quedaba huérfano y se purgaba: **deshacer el
    undo** se volvía imposible para siempre. Arreglado **reusando** la mecánica de H04
    (`write_pending_receipt`/`promote`/`discard`), no duplicándola.
- ✅ **E25-H06** — **El lock no tenía dueño demostrable**. `Drop` borraba el fichero **por ruta**:
  si otro proceso lo había reclamado por huérfano y recreado, el `Drop` del dueño original borraba el
  lock del **nuevo** dueño, y de ahí en cascada. El TTL de 15 minutos era wall-clock y reclamaba
  locks de dueños **vivos pero suspendidos** (un pid vivo se comprobaba, pero no mandaba), y la
  identidad no llevaba host, así que un pid de otra máquina se juzgaba como si fuera de esta. Ahora
  el lockfile lleva **token de propiedad** (el `Drop` solo borra si coincide) e identidad de máquina,
  y un pid vivo local impide el reclamo aunque el TTL haya vencido.
  - Y el `.gitignore` —el **único** fichero versionado del usuario que el motor toca, en cada
    `acquire_lock`— se reescribía con `std::fs::write` (no atómico) y reconstruido con `str::lines`,
    que **descarta los `\r`**: un `.gitignore` con CRLF se convertía a LF sin avisar. Ahora es
    atómico y preserva el estilo de fin de línea.
- ✅ **E26-H07** — **8 de 10 tools devolvían el código de error pelado**. No era descuido del
  despachador: los productores de `lodestar-app` eran `Result<_, ErrorCode>` y **no tenían dónde
  poner el mensaje**. El agente recibía literalmente `INVALID_SCHEMA`, sin qué parámetro ni qué se
  esperaba. Además `graph_query` sin `ref` respondía `DOCUMENT_NOT_FOUND` —el mismo error que si el
  documento no existe, con lo que el agente tomaba el camino de recuperación equivocado— y
  `change_plan` **descartaba** el `ParseError` que `knowledge_search` sí entregaba. Ahora las diez
  emiten `"CÓDIGO: mensaje"`, `graph_query` sin `ref` es `INVALID_SCHEMA` nombrando el parámetro, y
  el diagnóstico del parser sobrevive en las dos tools. **Sin tocar el catálogo**: sigue en 16 filas.
- ✅ **E26-H08** — **Un `TypeError` excluía documentos en silencio**. `if !matches!(evaluate(...),
  Ok(true)) { continue; }` metía el `Err` en el mismo cajón que el `Ok(false)`, en
  `knowledge_search` **y** en la selección masiva de `change_plan`. Consecuencia: una consulta con un
  error de tipo real devolvía una lista **recortada** e indistinguible de la correcta, decidida
  documento a documento; y en `change_plan` el plan afectaba a menos ficheros de los que el agente
  creía haber seleccionado. Ahora aborta con `INVALID_SCHEMA` nombrando campo, operador y los dos
  tipos, de forma **determinista** (el primer documento en el orden total ya existente). `Ok(false)`
  sigue siendo ausencia: no casar no es un error.
  - **Revisa dos rustdoc que consagraban lo contrario** («sin propagarse a la búsqueda entera», «sin
    abortar el plan»), igual que E24-H07 revisó el criterio de E19-H04.
- ✅ **E26-H09** — **Dos dialectos de dot-paths**. `metadata_inspect` normalizaba con
  `FieldPath::parse` en vez de `parse::build_field_path`, el punto único por el que pasan `where`,
  `filter` y `has`/`missing` desde E24-H07. Resultado: `graph.backlinks` significaba **dos cosas**
  según la tool; `frontmatter.graph.backlinks` —la sintaxis que el propio mensaje de error del parser
  recomienda— buscaba una clave literal `frontmatter` y devolvía `presentIn: 0`, silenciosamente
  equivocado; y el catálogo **anunciaba** nombres que ninguna consulta podía alcanzar, que es
  exactamente lo contrario de para lo que existe la tool. Ahora hay un solo normalizador
  (`build_field_path` pasa a público) y el `name` del catálogo es **direccionable**: se puede pasar
  tal cual a `mode:"field"` y a `where` y resuelve al mismo campo.
- ✅ **E26-H10** — **Respuestas sin cota**. `graph_query` no tenía default **ni máximo**
  (`None => total`): un `operation:"components"` servía el grafo completo. Y `metadata_inspect` era
  la única de las 10 tools sin `limit` ni `cursor`, con un `FieldStats` por field path —mapas
  intermedios incluidos— y un `ValueCount` por valor distinto, o sea **N entradas para N documentos**
  en un campo de alta cardinalidad. Ahora las dos tienen default y máximo y paginan con el mismo
  cursor-offset hex del resto de la superficie. La cota vive en la **fachada**: el core sigue puro y
  devolviendo la verdad completa, y los agregados (`presentIn`/`missingIn`) se computan sobre todo el
  workspace — se pagina la lista, no la estadística.
- ✅ **E26-H11** — **El contrato describía un servidor anterior**. `mcp.yml` seguía diciendo que un
  `where`/`filter` malformado daba «`WorkspaceError::Core` genérico» (comportamiento pre-E24-H10, y
  la propia cabecera del fichero ya documentaba lo contrario: se contradecía a sí mismo), **E24-H07
  declaró frontera sin tocar el contrato**, y cuatro tools declaraban sus errores como prosa suelta.
  Sincronizado con los cinco deltas de esta rama (E25-H02 y E26-H07…H10), con `/contrato --check`
  limpio y un test que **coteja** los códigos citados en el YAML contra `ErrorCode` — porque una
  lista de errores escrita a mano es justo lo que envejece en silencio (lección de E23-H13).

### Veredictos de los jueces ciegos (E25–E26)

Las **11 historias** pasaron por juez ciego (agente fresco, solo spec + diff) con *mutation testing*
pedido explícitamente en el encargo. **Las 11 volvieron `APROBADA CON RESERVAS`**, y **todas las
reservas MAYORES se cerraron en el mismo ciclo**: ninguna historia se dio por cerrada con una reserva
mayor viva. Las **menores** —flecos de **fuerza de suite**, casi todos mutantes supervivientes en
ramas de diagnóstico— quedan **declaradas** en [`decisiones §16(l)`](decisiones/16-deuda-auditoria-e25-e26.md#l-deuda-de-fuerza-de-suite-y-flecos-menores-registrados-por-los-jueces-ciegos), no cerradas:
ninguna describe un defecto observable hoy, y dos de ellas se arreglan con un refactor, no con un
test.

Lo que merece quedar registrado, porque cambió el plan:

- **Dos veces la épica ratificada tuvo que enmendarse durante la implementación**, las dos por un
  hallazgo del ciclo anterior, y las dos con commit de spec propio **antes** de la fase roja
  correspondiente: `2e0d6ea` (E25-H02: el aborto de ventana sella su journal, hallado implementando
  H01) y `cde2856` (E25-H05: revertir también deja recibo, **MAYOR-2** del juez de H04). Es la
  lección de E23 en su forma útil: la spec se corrige cuando implementar demuestra que estaba
  incompleta, no se implementa alrededor de ella.
- **Cuatro reservas no eran defectos de la historia sino deuda real del repo**, así que no se
  cerraron en código: se **declararon** en [`decisiones §16`](decisiones/16-deuda-auditoria-e25-e26.md) con su origen — los
  escritores de runtime sin lock (juez de H03), la duplicación de la secuencia de sellado
  `apply`/`revert` (juez de H05), los tres límites latentes de *quoting* del lenguaje (H09) y el
  cursor basura que reinicia en silencio (juez de H10). Los **flecos menores** de siete historias van
  aparte, en `§16(l)`, con el mutante que destapó cada uno.

### Invariantes que estas dos épicas dejan verificados

- **Único escritor (#5)**, reforzado: lo publicado **es** lo respaldado (H01), y el borrado es tan
  durable como la escritura (H05). Ninguna escritura del canónico esquiva ya `assert_writable`,
  backup y journal.
- **Crash-recovery**, ampliado de «nunca un `.md` a medias» a **«si el canónico cambió, existe el
  recibo que lo deshace»** — en el apply (H04) y en el revert (H05), y también tras `SIGKILL`.
- **Una sola verdad computada (#3)**: un solo dialecto de dot-paths en toda la superficie (H09), y la
  mecánica de recibo pendiente **reusada** en el revert en vez de duplicada (H05).
- **Un solo contrato de tipos (#4)**: el catálogo sigue en **16 filas**; lo único que se movió es que
  `RECOVERY_FAILED` ganó emisor, así que `codigos_sin_emisor` baja de 5 a 4.
- **Core puro (#2)**: intacto — las cotas de H10 viven en la fachada, no en `core::metadata`.

> **Recuento de tests**: esta rama añade la fase roja de las 11 historias al total de 486 de E24. La
> nota de release de la **v0.5.0** lo fijó en **541**, medido con
> `cargo test --workspace -- --list | grep -c ": test$"`, que es el criterio que E24-H18 dejó escrito
> para que este documento no vuelva a mentir con una cifra copiada. (No incluye los tests gateados
> tras `--features test-failpoints`, que ese comando no lista.)

## Producto, distribución y apertura OSS (E27) — COMPLETA (2026-08-02; H10 bloqueada)

> Primera épica de **superficie externa** (`ARCHITECTURE.md §21`, `decisiones §17`): el motor no
> cambia — cero deltas de contrato, cero cambios de comportamiento en `crates/*/src` (solo
> comentarios/doc-comments al mover docs). Origen: la review OSS externa del 2026-08-01, verificada
> punto a punto. Antes de la épica: retro-tag `v0.5.0` (el tag `0.5.0` sin prefijo nunca disparó
> `release.yml`; la release huérfana sin assets se borró tras verificar la nueva) y los quick fixes
> de README/RELEASING/fixtures (rama `chore/higiene-docs-release`, incluida en esta).

| Historia | Estado | Nota |
|---|---|---|
| E27-H01 guardarraíles de release | ✅ | `scripts/verifica-tag-release.sh` (3 casos ejecutados: v0.5.0→0, v0.6.0→1, 0.5.0→1) + `SHA256SUMS-<target>.txt` generado y verificado en el propio job. **Verificación diferida declarada**: la próxima release real (3 archivos + 3 checksums) la cierra. |
| E27-H03 `examples/demo/` | ✅ | 10 docs EN, enlace roto + huérfano deliberados y comentados; guion de 2 min con salidas reales. **Enmienda**: el huérfano se enseña vía `graph_query isolated` (el código `ORPHAN` murió en E16-H02), no vía `check`. |
| E27-H02 README en inglés | ✅ | Binarios de Releases + `cargo install --git` (probado de verdad), quickstart contra la demo, `claude mcp add` + JSON genérico, roadmap → `decisiones/`. Cero promesas de rendimiento (grep del criterio). |
| E27-H04 smoke de la demo | ✅ | `scripts/demo-smoke.sh` + job `demo-smoke` en `ci.yml`. Control anti-vacuo ejecutado (romper un enlace → falla). El smoke ya cazó una deriva real: añadir el README-guion cambió el blast radius 4/7→5/8. |
| E27-H06 `docs/` vigente vs superseded | ✅ | `docs/history/` con los 4 superseded (git mv, historia conservada), índice por audiencias, cero citas a rutas viejas (grep del criterio). `REFACTOR_PHASE_2.md` no se mueve (`§17`-DC). |
| E27-H05 `docs/user/` operativos | ✅ | quickstart/mcp-clients/ci en EN, todo ejecutado (parte con el binario de la release v0.5.0). El workflow de ejemplo de `ci.md` **se ejecutó en Actions** (run 30721903304, rama efímera): install → check → SARIF subido a code scanning → gate bloquea con exit 1, cada step como documenta. |
| E27-H11 `docs/user/` de referencia | ✅ | query-language/safe-changes en EN, ~45 tool calls, citas del binario verificadas verbatim (9/9), revisión cruzada contra `contracts/mcp.yml` declarada. Destapó los 3 hallazgos de `decisiones §19`. |
| E27-H07 `requirements/` veraz | ✅ | Banner HISTÓRICA en E0–E8, invariantes zombis #4/#7 corregidos, regla de idioma `§21.1` registrada, Done sin gates del frontend retirado. |
| E27-H08 comunidad | ✅ | CONTRIBUTING (issues-first `§17`-DB) + SECURITY (PVR **habilitado y verificado** + email) + Covenant 2.1 verbatim (diff: 1 línea, el contacto). Community profile se verifica tras el merge (lee `main`). |
| E27-H09 templates | ✅ | 3 formularios YAML válidos + PR template con los gates exactos del CI. Render de GitHub se comprueba tras el merge. |
| E27-H10 crates.io | ⛔ | **[BLOQUEADA por decisiones §17]** (diferida, reabrible). |

**Veredicto del juez ciego** (agente fresco, solo spec + diff, criterios re-ejecutados por él,
incluido el control anti-vacuo y ~30 llamadas MCP): **APROBADA CON RESERVAS (solo menores)** —
41/43 criterios ✓, 2 ± con enmienda declarada y justificada; ninguna reserva invalida una
historia. Las 2 reservas accionables se levantaron en el mismo ciclo (assert de `isolated` en el
paso 1 del smoke; constancia del `.gitignore` gestionado por el motor); las 2 restantes son
verificaciones post-merge declaradas (community profile / render de templates) y la end-to-end de
H01 (la cierra la próxima release).

**Hallazgos registrados al implementar** (regla de la épica: documentar ejecutando destapa, no
arregla): `decisiones §18` (`canApply: false` no vincula a `change_apply`) y `§19` (a: `has(frontmatter)`
nunca casa, contradice `§20.8`; b: `policy` parcial rechazada pese a campos opcionales del contrato;
c: imprecisión de `§16.a` caso 3). Todos tocan la frontera o el core → historias propias fuera de E27.

## Fase 0 de la campaña de bugfixes del testbench homelab (E28) — COMPLETA

> **Origen**: `decisiones/23-hallazgos-testbench-homelab.md` (M-01 y A-05, prioridades 5 y 4 — las
> dos filas con **riesgo real de pérdida de conocimiento**, ejecutadas antes que cualquier otro
> hallazgo de la tabla) y `docs/qa/informe-homelab-2026-08-06.md` (caso G1-18 para M-01, caso G1-11
> para A-05). Épica: [`epica-28-defectos-destructivos-testbench.md`](requirements/epica-28-defectos-destructivos-testbench.md).
> Tablero de campaña: [`docs/qa/campana-bugfixes-2026-08.md`](docs/qa/campana-bugfixes-2026-08.md).
> Commits: `043f233` (H02), `296147b` (H01), `8c86b6b` (adenda H03+H04), `c532929` (cierre de
> reservas de los re-jueces) — los cuatro en `develop`, **pendiente de merge a `main`**.

| Historia | Estado | Detalle |
|---|---|---|
| **E28-H01** `change_revert` de un `-revert` restaura de verdad | ✅ | Identidad propia del `txnId` de cada reversión + coreografía de sellado unificada (`decisiones §16(i)`). |
| **E28-H02** guard de colisión en `create`/`move` | ✅ | `DOCUMENT_ALREADY_EXISTS`, catálogo `ErrorCode` 16→17. |
| **E28-H03** identidad de transacción libre en la publicación (adenda) | ✅ | Corrige el bloqueante que el juez de H01 dejó en `change_apply`. |
| **E28-H04** normalización contra el estado acumulado del change set (adenda) | ✅ | Corrige el bloqueante que el juez de H02 dejó en las colisiones intra-plan. |

- ✅ **E28-H01** (`296147b`) — **Revertir un recibo `-revert` era un no-op silencioso que destruía el
  redo**. La identidad de una reversión se derivaba del `changeSetId` heredado, que colisionaba
  consigo misma: `revert(revert(X))` recalculaba el mismo `txnId` `X-revert` que el primer revert,
  así que restauraba un árbol que ya estaba vigente (no-op) y sobrescribía en silencio
  `recovery/X-revert/` —la única copia del estado que el primer revert había dejado— sin que ningún
  error lo señalara. Ahora `revert_transaction_id` deriva la identidad del **`receiptId`** que se
  revierte, no del `changeSetId`, apilando un contador (`-revert`, `-revert-2`, …) que compone sin
  límite. Guard anti-sobrescritura en `revert_transaction_con_recibo` con el mismo criterio de
  «vivo» que usa el GC (`journal ∪ receipts`). De paso satura `decisiones §16(i)`: la coreografía
  de sellado duplicada entre `apply_transaction_con_recibo` y `revert_transaction_con_recibo` se
  extrae a `seal_published_transaction`, un único camino compartido.
- ✅ **E28-H02** (`043f233`) — **`create`/`move` sobre un destino ocupado aplicaban sin fricción y
  pisaban conocimiento existente**. `normalize_create` descartaba el `DocumentSet` (literalmente
  `_workspace`) y `normalize_move` nunca consultaba `doc_set.files()` para el destino `to`. Ahora
  los dos comprueban la ocupación del path y fallan con el código nuevo `DOCUMENT_ALREADY_EXISTS`
  (catálogo `ErrorCode` 16→17, simétrico de `DOCUMENT_NOT_FOUND`), nombrando el path colisionado;
  `move` con `from == to` sigue siendo no-op válido, no una colisión. Delta de contrato declarado en
  `contracts/mcp.yml`.
- ✅ **E28-H03** (`8c86b6b`) — **Bloqueante que el juez ciego de H01 dejó vivo**: el guard
  anti-sobrescritura de H01 solo protegía `change_revert`; `change_apply` seguía calculando
  `txn_id = transaction_id(&change_set.id)` sin pasar por él, así que replanificar el mismo cambio
  (mismo `changeSetId` determinista) y volver a aplicarlo **sobrescribía** `recovery/`/`receipts/`
  de la primera transacción, y el `revert` posterior quedaba **sin salida** (`WRITE_CONFLICT`
  permanente, el `txnId` de su propia reversión ya estaba tomado). Ahora `resolve_free_txn_id`
  resuelve la identidad efectiva de **ambos** caminos buscando de forma determinista la primera
  variante libre (mismo criterio `journal ∪ receipts`) antes de la primera escritura:
  `apply → revert → re-apply idéntico → revert` completa con **cuatro `receiptId` únicos**, sin
  pisar nunca material previo. `revert_transaction_id` endurece el sufijo al formato canónico y su
  rustdoc declara explícitamente el borde `u64::MAX`.
- ✅ **E28-H04** (`8c86b6b`) — **Bloqueante que el juez ciego de H02 dejó vivo**: los guards de H02
  normalizaban cada operación del plan contra el `DocumentSet` **inicial**, así que colisiones
  reales **dentro** de un mismo plan (`[move a→final, move b→final]`, `[create X, move b→X]`,
  `[create X, create X]`) no se veían — falsos negativos destructivos — mientras que idiomas
  legítimos que dependían de la secuencia (`[delete X, create X]`, `[move A→B, create A]`) se
  rechazaban por error — regresión respecto al commit padre de H02. Ahora la normalización lleva un
  estado de ocupación acumulado (`EstadoOcupacion`: `create`/`move.to` ocupan, `delete`/`move.from`
  liberan) que cada operación del plan actualiza en orden, con el mismo juicio de colisión que el
  guard contra disco (invariante #3, una sola verdad computada). Abre `decisiones §24`
  (equivalencia de paths por caja/Unicode), explícitamente fuera de su alcance.
- **Cierre de reservas de los re-jueces** (`c532929`) — la red de colisiones intra-plan pasa a ser
  **vinculante en `change_apply`** (el plan persistido es un artefacto durable que pudo escribirse
  con un binario sin el guard; la de `change_plan` queda como diagnóstico temprano, ambas llaman al
  mismo juicio del core); el motivo del `WRITE_CONFLICT` residual deja de filtrar rutas del plano de
  control (regla fijada en rustdoc: el motivo lo lee un agente); contrato sincronizado
  (`DOCUMENT_ALREADY_EXISTS` declarado en `change_apply`, los cuatro emisores de `WRITE_CONFLICT` en
  `change_revert` incluidos los dos bordes `u64::MAX`, el no-op `move` `from == to` deja su path
  ocupado en el acumulado); y dos familias de defecto preexistentes (resurrección de paths por
  operaciones de contenido, move-chains por ocupación del origen) quedan registradas como
  seguimiento, fuera de esta épica, en la sección «Hallazgos preexistentes registrados» de la spec.

### Veredictos de los jueces ciegos (E28)

**H01 y H02** pasaron cada una por **panel de 3 jueces ciegos** (agentes frescos, solo spec + diff,
tres lentes distintas). Cada panel localizó un bloqueante real **ejecutando el binario por
JSON-RPC** —la misma disciplina que produjo el hallazgo original—: el de H01 en el camino de
`change_apply` (cerrado por H03); el de H02 en la normalización intra-plan (cerrado por H04). Tras
la adenda, **re-jueces ciegos de robustez** verificaron H03/H04 con el mismo método y devolvieron
**APROBADA CON RESERVAS**; las reservas se saldaron en el mismo ciclo (`c532929`), sin ninguna
viva. `/contrato --check` quedó **COHERENTE en dos pasadas** (tras la adenda y tras el cierre de
reservas).

### Invariantes que esta épica deja verificados

- **Suite completa en verde**, incluidos ambos crates con `--features test-failpoints`
  (`lodestar-workspace` y `lodestar-app`) — 583+ tests tras las cuatro historias.
- **Única fuente de verdad (#1) y único escritor (#5)**: ninguna transacción de publicación
  (`apply` o `revert`) puede ya pisar `recovery/`/`receipts/` de una transacción con material
  vigente; la identidad se resuelve buscando, no sobrescribiendo ni fallando sin salida.
- **Una sola verdad computada (#3)**: el guard de colisión de `create`/`move` juzga con el mismo
  criterio contra disco y contra el estado acumulado del propio plan; la coreografía de sellado de
  `apply`/`revert` vive en un único camino compartido (`seal_published_transaction`).
- **Un solo contrato de tipos (#4)**: catálogo `ErrorCode` en 17 filas (16→17,
  `DOCUMENT_ALREADY_EXISTS`), delta declarado en `contracts/mcp.yml` y verificado por
  `/contrato --check` en dos pasadas.
- **clippy `-D warnings`, `cargo fmt --check` y `cargo doc` limpios** en los cuatro commits.

**Estado real**: las cuatro historias están implementadas, verificadas por jueces ciegos y con la
suite en verde en `develop` — la épica está **completa a falta del merge**: los cuatro commits ya
viven en `develop`, pero aún no cruzaron a `main` por el ciclo de release descrito en
`RELEASING.md`.

## Fase 1 de la campaña de bugfixes del testbench homelab (E29 — honestidad de superficie) — COMPLETA

> **Origen**: `decisiones/23-hallazgos-testbench-homelab.md` (D-01, A-04, A-07) + el orden de trabajo
> de `decisiones/README.md` (punto 1: §19a/b, §18 vinculante, §16f, §16e, §15, §16g, §16b). Épica:
> [`epica-29-honestidad-superficie.md`](requirements/epica-29-honestidad-superficie.md). Tablero de
> campaña: [`docs/qa/campana-bugfixes-2026-08.md`](docs/qa/campana-bugfixes-2026-08.md). Rama
> `feat/e29-honestidad-superficie`, **11/11 historias implementadas, pendiente de merge**.

| Historia | Commits | Veredicto del juez | Detalle |
|---|---|---|---|
| **E29-H01** config estricta (`§16e`+A-08) | `4a52f59` | APROBADA (8/8) | `deny_unknown_fields` en `WorkspaceConfig` y sus secciones; familias de `validation` contra la lista cerrada `VALIDATION_FAMILIES`; config ilegible → `Err` (exit 3), no *defaults* silenciosos. |
| **E29-H02** `policy` parcial respeta el `Default` | `46c1492` + `2f6ecf4` (remate del juez) | APROBADA (5/5) | Campos de `PlanPolicy` con `#[serde(default)]`; el remate clava literales y el default por campo omitido. |
| **E29-H03** `has`/`missing` responden la verdad | `99900d3` | APROBADA (7/7) | El anclaje pelado (`has(frontmatter)`) deja de ser el único argumento donde ambos operadores mienten. |
| **E29-H04** afijo sobre no-string es type error | `b3b79fb` + `681ec45` (remate: mensaje neutro) | APROBADA (7/7) | `starts_with`/`ends_with` sobre un campo no-string producen el mismo type error ruidoso que E26-H08 fijó para el orden. |
| **E29-H05** scope `paths` exige existencia | `fc5c26b` | APROBADA (6/6) | Un path inexistente en el scope `paths` de `knowledge_check` responde `DOCUMENT_NOT_FOUND`, simétrico con `document`/`affected`. |
| **E29-H06** workspace vacío avisa | `88e99b2` + `6a3a6ca` (remate: mensaje, ancla, productor con red) | aprobada | `WORKSPACE-EMPTY` (warn) cuando la raíz no descubre ningún documento; no toca exit codes. |
| **E29-H09** `instructions`/`protocolVersion` coherentes | `5e7edc0` | APROBADA (7/7) | `instructions` nombra exactamente las tools que el perfil sirve; `protocolVersion` no soportada deja de aceptarse en silencio. |
| **E29-H07** `canApply` vinculante | `9df617f` + `f97004c` (remate: doc deja de negar el gate, pines fijan la representación) | aprobada (bloqueante doc saldado) | `change_apply` rechaza un plan con `canApply: false` bajo su propia policy. |
| **E29-H08** wire estricto | `f7dc5fd` + `f720ba8` (remate: deuda saldada, cascada a sub-objetos) | APROBADA (11/11) | `additionalProperties: false` se ejecuta de verdad (validación por unión contra la tabla de campos legales por operación de `§15`). |
| **E29-H10** repliegue de la API no transaccional | `7f519d2` | juez ciego: APROBADA | `create_document`/`write_document`/`merge_frontmatter`/`publish` replegadas a `pub(crate)` (tests vía feature `test-support`, verificado con consumidor externo). |
| **E29-H11** retirada del `Envelope` | `7f519d2` | juez ciego: APROBADA | `Envelope<T>`/`ErrorEnvelope`/`ResourceLink` de `lodestar-app` retirados (§16(b): capacidad construida en E10-H01/H02 sin consumidor real; cero residuos verificados). |

**Invariantes verificados**: suite completa en verde, incluidos los dos crates con
`--features test-failpoints` (`lodestar-workspace` y `lodestar-app`); `cargo clippy --workspace
--all-targets --all-features --locked -- -D warnings` limpio; `cargo fmt --all --check` limpio;
`cargo doc --workspace --no-deps --locked` limpio; pureza del core intacta (`cargo tree -p
lodestar-core` sin tokio/rusqlite/git2/notify/tauri/zip). `/contrato --check` coherente tras cada
historia que tocó la frontera (H01, H02, H04, H05, H06, H07, H08, H09).

**Estado real**: las 11 historias están implementadas y **mergeadas** (PR #27, merge `fb48c03`), con
todas las reservas MAYORES/bloqueantes de sus jueces ciegos saldadas en los commits de remate (H02,
H04, H06, H07, H08) y H10/H11 aprobadas por juez ciego (`1c09af3`).

## Fases 2-3 de la campaña de bugfixes del testbench homelab (E30 — higiene y escoba) — COMPLETA (pendiente de merge)

Épica cerrada el **2026-08-07** (`requirements/epica-30-higiene-escoba.md`, ratificada por el
usuario). Cierra el **ciclo de higiene** de `decisiones §16(j)/(l)` ampliado con `§23/A-02,A-03`, la
flakiness del lock que tres jueces habían señalado, y la **escoba documental** de los nits de `§23`
más los seguimientos acumulados durante las Fases 0 y 1. Con ella, `decisiones §23` queda **cerrada**:
sus 12 subpuntos accionables están ejecutados.

| Historia | Commits | Juez ciego | Qué cierra |
|---|---|---|---|
| **E30-H01** cursores estrictos y firmados | `8359294` + `2d32eeb` (remate de robustez) | RECHAZADA → saldada | El cursor va firmado con su origen: malformado, retocado o **de otra tool** se rechaza con `INVALID_SCHEMA` en vez de servir la página 1 (`§16(j)` + `§23/A-02,A-03`). |
| **E30-H02** publicación atómica del lock | `9cd129a` + `6b5c2b7` (4 remates) | APROBADA con reservas, saldadas | La flakiness de `crash_por_senal_no_deja_parciales` **no era un test frágil: era un bug real** — un `SIGKILL` en la ventana no atómica dejaba un lock vacío y **terminal**, cerrando el workspace a la escritura para siempre. |
| **E30-H03** escoba: 3 defectos + 11 criterios documentales | `0ef66d2` + `8621e40` (3 MAYOR) | APROBADA CON RESERVAS (9/11), saldadas | `contains` con literal no-string → type error; `protocolVersion` no-string → `-32602`; prefijo duplicado de `INVALID_RESULT`; D-02/A-01/A-06/A-09/A-10 y los seguimientos de SARIF y `counts`. |
| **§16(l)** pasada de mutantes | `9d09c62` | n/a (higiene de suite) | 6 supervivientes confirmados y muertos con test. Dos revelaban **diagnósticos que mentían** (el mensaje de cuarentena decía «nada se ha borrado» mientras borraba el sidecar). |

**Lo que la ejecución corrigió sobre lo que la campaña suponía** — se registra porque es el valor
real del dogfooding, y porque repite la lección de E23 (*leer el código no basta: ejecútalo*):

- **La firma de cursores introdujo una regresión de robustez que la suite no vio**: un cursor
  no-ASCII (`«🔥.807e307a»`) hacía `panic!` por un corte fuera de frontera de carácter y se llevaba
  la **sesión JSON-RPC entera** (rc=101), en las cuatro tools paginadas. Lo cazó el juez ciego de
  robustez ejecutando el binario, no los tests.
- **Los tres tests preexistentes del reclamo de locks dejaban pasar la mutación `&&`→`||`**, que
  reclama locks de dueños vivos. Es decir: cubrían el camino, no la frontera.
- **Una hipótesis registrada era falsa.** La divergencia de `workspace_status.counts` se había
  anotado como «diagnósticos sin target»; verificarla la refutó (`SYMLINK-UNSUPPORTED` y
  `DOC-NOT-UTF8` sí llevan targets y divergen igual). El criterio real es que el fichero nunca entra
  al inventario.
- **A-06 era la punta de otro defecto**: un `replace_text` que no casa nada **reescribe el fichero**
  y reserializa el frontmatter. Fuera del alcance de H03 por causa raíz distinta; **abierto**.
- **Un test aseveraba la negación del defecto que su propio commit documentaba** (el de guardia de
  A-06), y pasaba solo porque el fixture no tenía frontmatter en estilo flow. El juez lo demostró
  añadiendo `tags: [a, b]`. Corregido con fixture propio de dos documentos, donde el de estilo block
  es el anti-vacuo.
- **Dos arreglos llegaron sin red**: el de SARIF sin un solo test (neutralizar el guard dejaba los 51
  tests del crate en verde) y una costura extraída «para que sea ejercitable» que **nadie ejerció**.
- **La pasada de mutantes destapó superficie pública muerta**: `Workspace::revert_transaction` con el
  cuerpo en `unreachable!()` deja los 52 binarios de test del workspace en verde. Registrado en
  `decisiones §16`, **sin actuar**: es la categoría de `§16(b)`/`§16(g)`, que se resolvieron por
  decisión de retirar, no añadiendo tests.

**La lección**: los cuatro fallos de arriba convivían con la suite **en verde**. E23 dejó escrito
*«cuando dudes de si algo funciona, ejecútalo»*; E30 añade el piso de arriba — **cuando dudes de si
un test muerde, mútalo**.

## Los dos seguimientos de la campaña (E31) — COMPLETA (2026-08-08)

Épica: [`requirements/epica-31-seguimientos-campana.md`](requirements/epica-31-seguimientos-campana.md).
Cierra las dos fichas que la campaña dejó abiertas porque exigían criterio del usuario:
[`§25`](decisiones/25-superficie-muerta-revert-transaction.md) y
[`§26`](decisiones/26-replace-text-noop-reserializa.md).

| Historia | Qué cierra | Estado |
|---|---|---|
| E31-H01 | `§25` — `Workspace::revert_transaction` se **retira** | ✅ |
| E31-H02 | `§26` — la cabecera se preserva byte a byte + `noOpOperations` | ✅ |

- **H01 ejecutó la salida que la ficha desaconsejaba, y la decidió el compilador.** `§25` recomendaba
  replegar a `pub(crate)` por reversible; al hacerlo, **clippy la marcó como `dead_code`** —no la
  usaba nadie tampoco dentro del crate— y con el CI en `-D warnings` el repliegue era
  **incompilable**. Con `pub` el aviso no salía porque una función pública puede tener consumidores
  externos: replegarla fue lo que lo destapó. El argumento de la ficha para preferir el repliegue
  («es el cuerpo que `revert_transaction_con_recibo` envuelve») era además **falso**: era un wrapper
  de tres líneas sobre ella. Lo único que costó fue trasladar su doc-comment, el único sitio donde
  vivía la descripción de la mecánica de reversión.

- **H02 destapó que `§26` eran TRES defectos, no uno.** La ficha reportaba el frontmatter
  reserializado (`tags: [a, b]` → estilo bloque). El brazo `ReplaceBody` perdía además el
  **separador** (normalizado a `---\n\n`, o sea una línea en blanco inyectada en cada reescritura) y
  —lo grave— **borraba entero el frontmatter ILEGIBLE**: `parse_file` devuelve `None` tanto para «no
  hay bloque» como para «hay bloque y su YAML no se deja leer», así que reescribir el **cuerpo** de
  un documento con la cabecera rota se llevaba la cabecera por delante. **Pérdida de datos, sin un
  solo test** que combinara frontmatter ilegible con una operación de cuerpo.

  Los tres caen con el mismo arreglo —`model::replace_body_preservando_cabecera`, inverso exacto de
  `SplitFront::body`— porque corta por **posición de bytes** y no por si el YAML se interpreta. El
  radio era todo lo que normaliza a `ReplaceBody`: `replace_text`, `edit_section`, `replace_body`,
  `delete remove_links` y `move` — incluido el `rewriteInboundLinks`, que reescribe el cuerpo de
  **cada enlazante**, así que un `move` podía reformatear medio workspace.

- **La ampliación sobre la ficha**: `PlanResult.noOpOperations`. Con el churn eliminado, una
  operación sin efecto no dejaba **ninguna** traza, y el agente no podía distinguir «se procesaron 40
  y 28 no casaron» de «solo se seleccionaron 12». `docs/user/safe-changes.md` ya nombraba el hueco
  por escrito: *«no field saying "zero replacements"»*. Va fuera del `planHash` —el hash es la
  identidad de lo que se **pidió**; «resultó no-op» es una propiedad del **resultado**— y la
  operación **no** se elimina del plan.

### Lo que esta épica añade al método

- **El riesgo que no se podía cerrar leyendo, se cerró ejecutando ANTES de implementar.** Con el
  arreglo, un no-op deja de tocar disco y la transacción publica un lote de **cero paths** — camino
  sin ningún guard. Se escribió `transaccion_con_lote_vacio_no_degenera` primero: aplicar y revertir
  un lote vacío funcionan. Y el primer intento de ese test **falló porque no conseguía producir un
  lote vacío**, lo que reprodujo `§26` aislado en la capa de transacción, tres capas por debajo de
  donde lo reportó el juez ciego.
- **Dos tests preexistentes fijaban el defecto sin saberlo** y cambiaron de expectativa (no de
  intención): `una_promocion_de_recibo_fallida_conserva_el_journal` y
  `recovery_sin_parciales_por_el_orquestador_real` daban por buena la línea en blanco inyectada. El
  segundo construía su borde esperado sustituyendo texto sobre el original, así que **solo casaba
  gracias al defecto**.
- **El fixture del test invertido dejó de ser anti-vacuo al arreglarse el bug.** Tras preservar la
  cabecera, sus dos documentos se comportan igual, así que un motor que no escribiera **nunca** nada
  habría pasado el test entero; hizo falta un tercer documento sobre el que la operación **sí** casa.
  Es el mismo modo de fallo que originó `§26` (un test verde que no probaba nada), reapareciendo al
  cerrarla.
- **Verificado por el wire real**, que es como se destapó: `replace_text` sin coincidencias sobre un
  `.md` con `tags: [atlas, overview]` da `modified: []`, `noOpOperations: [{index: 0, op:
  "replace_body", path: "overview.md"}]` y **md5 idéntico**; uno que sí casa cambia el cuerpo y deja
  el frontmatter en estilo flow.
- **El PR creó la misma superficie muerta que retiraba** — lo cazó el juez ciego de H02 mutando el
  parámetro a inerte y viendo los 740 tests en verde. Al dejar `ReplaceBody` de reconstruir
  documentos, `build_raw_with_bom` se quedó sin un llamador que pasara `bom: true`; se fusionó con
  `build_raw`. La ironía es la lección: H01 retira superficie muerta **en el mismo rango de commits**
  en que H02 la crea, y ninguna suite lo habría notado.
- **Un test escrito para fijar un límite midió otra cosa.** El límite por-documento de
  `noOpOperations` se iba a fijar con «dos ops sobre el mismo path, solo una vacía → ninguna se
  declara». La realidad medida fue peor: la segunda op se normaliza contra el estado **inicial**
  (defecto preexistente de la normalización multi-op), deshace a la primera, y el documento acaba
  idéntico, así que **ambas** se declaran. El test fija lo medido, no lo supuesto.
- **La documentación prometía de más sobre `edit_section`**: contrato y `docs/user/` afirmaban que el
  campo cubre «reescribir una sección con su contenido actual». Verificado por el wire: depende de la
  forma del documento, porque `normalize_edit_section` fija la separación de la sección. Prometer lo
  que el binario no cumple es el modo de fallo que **originó** `§26`, reapareciendo al cerrarla.

## Defectos posteriores a E27

| Defecto | Estado | Nota |
|---|---|---|
| `outputSchema` de `metadata_inspect` sin `type: "object"` | ✅ | Claude Code **rechazaba la lista completa de tools**. |

- ✅ **`outputSchema` no conforme al spec MCP** — el spec exige que todo `outputSchema` sea un JSON
  Schema **de tipo `object`**; el de `metadata_inspect` salía con `anyOf` en la raíz y **sin
  `type`**. Un cliente estricto no degrada la tool inválida: **rechaza la lista entera**, así que
  Claude Code no registraba ninguna de las 10 y el motor era inusable desde ese cliente — el defecto
  tenía radio de servidor, no de tool.

  **Causa raíz**: los `outputSchema` se derivan con `schemars` del tipo Rust real de cada servicio
  (D6b, `ARCHITECTURE.md §10` fila 6). Nueve salidas son `struct` y schemars les emite
  `type: "object"` gratis; `MetadataInspection` es el **único `enum`** de la superficie y lleva
  `#[serde(untagged)]`, que schemars traduce a un `anyOf` de las variantes sin inferir `type` en la
  raíz (en el caso general las ramas podrían ser de tipos JSON distintos). El arreglo lo fija en
  `schemas::metadata_inspect_schema()`, **sin tocar el wire**: las dos variantes ya eran objetos, así
  que declararlo en la raíz no excluye ninguna respuesta válida.

  **Por qué la suite no mordía** (el patrón de E23, otra vez): la invariante «la raíz es un objeto»
  estaba escrita para el input —`tools_list_lleva_input_schema`, y sobre las 10— y **nunca para el
  output**. El `tools_declaran_outputschema` de E10-H13 miraba **5** de las 10 y daba por bueno que
  apareciera cualquier clave estructural, con **`anyOf` explícitamente en su allowlist**: pasaba en
  verde sobre el schema roto, no por accidente sino porque su criterio solo pedía «parece un JSON
  Schema». El `structured_content_conforma_output_schema` de E24-H15 tampoco podía verlo: mide que la
  salida **conforma** su schema declarado, y un `anyOf` sin `type` conforma perfectamente.

  **Verificado**: el defecto se reprodujo por stdio (`tools/list` contra el binario) antes de tocar
  nada, y las dos guardias nuevas se vieron **en rojo** revirtiendo el arreglo. Hoy vigilan la
  invariante en las 10 tools `tools.rs::tools_list_lleva_output_schema_de_tipo_object` (en proceso) y
  el `tools_declaran_outputschema` endurecido (e2e). La regla queda escrita en `contracts/mcp.yml`,
  que hasta ahora no la exigía en ninguna parte.
