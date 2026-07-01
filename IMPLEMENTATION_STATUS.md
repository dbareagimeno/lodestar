# Estado de implementación

> Mapea las épicas/historias de [`requirements/`](requirements/) a su estado real en esta rama.
> Construido en el **orden de fases ratificado** (`ARCHITECTURE.md §14`), validando con tests en cada fase.
>
> **Resumen**: el **backend está completo y testeado** (core + store + vcs + workspace + CLI + MCP).
> El store (SQLite/FTS5 + watcher) está implementado con paridad SQL==core; el bus de eventos en vivo
> está cableado en la workspace; los cierres de git (check --staged/--rev, switch/merge, hooks, cache de
> conformidad por tree-oid) están hechos; config por-bundle (`lodestar.toml`), import del prototipo e
> `init` con git real están hechos. Queda **E6 desktop** (fachada Tauri + port verbatim de la UI, ver
> [`DECISIONES.md`](DECISIONES.md)). **~85 tests** en verde; `cargo clippy --workspace --all-targets --
> -D warnings` limpio; `npm run build`/`check` del frontend en verde.

## Cómo correrlo

```bash
cargo test --workspace          # 46 tests (core, cli, vcs, workspace, mcp)
cargo run -p lodestar-cli -- check --path <bundle>     # la puerta de CI (exit 0/1)
cargo run -p lodestar-cli -- log | last-conforming | branch
cargo run -p lodestar-mcp -- <bundle>                  # servidor MCP por stdio
cd frontend && npm install && npm run build            # frontend Svelte 5 → dist/
```

## Estado por épica

| Épica | Estado | Detalle |
|---|---|---|
| **E0** Scaffolding | ✅ Hecho | Cargo workspace con 7 crates + direcciones del §3; `#![forbid(unsafe_code)]` en core; fixtures; CI (fmt/clippy/test + frontend); frontend Svelte/Vite. |
| **E1** `lodestar-core` | ✅ Hecho | Contrato de tipos congelado, modelo, conformidad (15 checks + OKF-CONFLICT), analyze, query, grafo, generadores, export, diff. 22 tests. |
| **E2** `lodestar-cli` | ✅ Hecho | `check` (humano/--json/--sarif), `index`/`tags` (--check→exit 4), `export`, `init`; exit codes congelados. 8 tests. |
| **E3** `lodestar-store` | ✅ Hecho | DDL dueño único (`files`/`links`/`tags`/`diagnostics` + FTS5 + `commit_conformance`), cold rebuild, watcher `notify-debouncer-full` con **gate por hash blake3**, síntesis SQL (backlinks/orphans/dangling/blast-radius CTE), FTS5 con escapado, bus `IndexEvent` (crossbeam), trait `ConceptStore`. **13 tests**: paridad SQL==core, property incremental==core (120 ediciones), watcher en vivo, FTS. |
| **E4** `lodestar-vcs` | ✅ Hecho | libgit2 local + red por binario `git` + **resolve_rev**, **staged_files**, **switch** (sin tocar working tree), **merge** (3-vías a nivel de árbol con marcadores + `MERGE_HEAD`), **install_hooks**, **tree_oid**. Cache de conformidad por tree-oid en el store, cableada en la workspace. **12 tests**. |
| **E5** `lodestar-workspace` | ✅ Hecho | Handle unificado, único escritor, snapshot, commit/restore con checkpoint, switch/merge, conformidad cacheada por tree-oid, config (`lodestar.toml`), y **bus de eventos en vivo** (`open_live`/`enable_cache`/`subscribe`) con **update optimista** de la cache tras cada escritura. **12 tests**. |
| **E6** Tauri + frontend | 🟢 Parcial | Frontend Svelte 5: consume el `BundleSnapshot` (árbol filtrable + selección + panel de conformidad **localizado** i18n keyed por código), listener `bundle:changed`, CSS portada. Compila (`npm run build`) y pasa `svelte-check`. `src-tauri` sigue placeholder (Tauri necesita libs de sistema → ver [`DECISIONES.md §1`](DECISIONES.md)). Pendiente: fachada Tauri + port verbatim (editor/grafo/overlay/Cambios). |
| **E7** `lodestar-mcp` | 🟢 Parcial | 13 tools sobre la workspace + bucle JSON-RPC por stdio (stdout puro). **Golden cross-fachada** (tool==workspace) + e2e. **5 tests**. Pendiente: transporte `rmcp` oficial + resources (ver [`DECISIONES.md §3`](DECISIONES.md)). |
| **E8** Transversales | 🟢 Parcial | Hechos: exit codes/SARIF, escritura atómica, **zip-slip cerrado por RelPath en `import`**, identidad de commits + override por `lodestar.toml`, trailer Co-Authored-By del agente, gitignore de `.lodestar/`, **config por-bundle (`lodestar.toml`: strictness + identidad)**, **`lodestar import`** (zip del prototipo o dir), **`init` con git init + commit inicial real**, **i18n keyed por código** (catálogo español). Pendiente: packaging/updater, gate de bench (§11), threat model, arnés diferencial JS-vs-Rust (ver [`DECISIONES.md §9`](DECISIONES.md)). |

## Cobertura de historias (destacadas)

- **E1**: H01–H17, H19 implementadas y testeadas; H18 (arnés diferencial JS-vs-Rust) **no**: los tests del core
  fijan la semántica del prototipo directamente en Rust en vez de ejecutar el JS en Node. H20 (schemars/render)
  como features.
- **E2**: H01–H05 hechas; H06/H07 (reindex/import/git) reales o stub según fase.
- **E4**: H01–H06, H09 (conformidad por commit) hechas; H07 (red) hecha; H08/H10 parciales.
- **E5**: H01–H06 hechas (sin el watcher de E3); H07 parcial.
- **E7**: H01–H05 hechas (subset stdio); H06 (golden cross-fachada) **pendiente**; H07 doc.

## Invariantes verificados

- **Core puro**: `lodestar-core` no declara `tauri`/`rusqlite`/`notify`/`tokio`/`git2`; `#![forbid(unsafe_code)]`.
- **Una sola verdad computada**: la conformidad por commit (vcs) y el gate (cli) usan el **mismo** `core::analyze`.
- **Un solo contrato de tipos**: definido una vez en `core::types`; el front lo refleja (a generar con ts-rs).
- **RelPath**: newtype validado; único chokepoint de path-traversal (tests de absolutas/`..`).
- **git vocabulario directo + transporte híbrido**: libgit2 local, binario `git` solo para red.
- **Único escritor**: la workspace escribe `.md` atómico (temp+rename); nadie más escribe.

## Próximos pasos (orden sugerido)

Todo lo que queda depende de decisiones tuyas — ver [`DECISIONES.md`](DECISIONES.md):

1. **E6** desktop: aislar el build de Tauri (§1) → cablear la fachada + port verbatim de la UI (§2).
2. **E0-H04/E6-H03**: generar el `.d.ts` desde Rust (ts-rs/specta) antes de crecer la UI (§4).
3. **E7**: adoptar `rmcp` oficial + resources cuando haya un cliente que lo exija (§3).
4. **E8**: gate de bench (§11), packaging/updater + firma, threat model, arnés diferencial JS-vs-Rust (§9).
