# Estado de implementación

> Mapea las épicas/historias de [`requirements/`](requirements/) a su estado real en esta rama.
> Construido en el **orden de fases ratificado** (`ARCHITECTURE.md §14`), validando con tests en cada fase.
>
> **Resumen**: **todas las épicas (E0–E8) están implementadas y verificadas.** Backend completo
> (core + store SQLite/FTS5+watcher con paridad SQL==core + vcs con switch/merge/hooks + workspace con
> bus en vivo + CLI + MCP con golden cross-fachada) y **escritorio completo** (fachada Tauri v2 con la
> tabla de comandos congelados + evento `bundle:changed`, y UI Svelte 5 funcional: árbol, editor
> multi-escritor, isla del grafo, modo Cambios). **~91 tests** en verde; `cargo clippy --workspace
> --all-targets --all-features --locked -- -D warnings` limpio; `cargo doc -D warnings` limpio;
> `npm run check`/`build` del frontend en verde. Lo pendiente es **producto/pulido**, no arquitectura
> (empaquetado/firma, rails redimensionables, rmcp, `.d.ts` generado): ver [`DECISIONES.md`](DECISIONES.md).

## Cómo correrlo

```bash
cargo test --workspace          # ~91 tests (incl. 6 diferenciales JS-vs-Rust; core, store, cli, vcs, workspace, mcp)
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
| **E8** Transversales | 🟢 Parcial | Hechos: exit codes/SARIF, escritura atómica, **zip-slip cerrado por RelPath en `import`**, identidad de commits + override por `lodestar.toml`, trailer Co-Authored-By del agente, gitignore de `.lodestar/`, **config por-bundle (`lodestar.toml`: strictness + identidad)**, **`lodestar import`** (zip del prototipo o dir), **`init` con git init + commit inicial real**, **i18n keyed por código** (catálogo español), **arnés diferencial JS-vs-Rust (§12)**. Pendiente: packaging/updater, gate de bench (§11), threat model. |

## Cobertura de historias (destacadas)

- **E1**: H01–H19 implementadas y testeadas, **incluida H18** (arnés diferencial JS-vs-Rust:
  `prototype/harness/` corre las funciones puras del prototipo en node y `tests/differential.rs` compara con
  el core sobre 6 fixtures — analyze · query · generadores · grafo, con el prototipo como oráculo). H20
  (schemars/render) como features.
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

## Próximos pasos (todo opcional — producto/pulido, ver [`DECISIONES.md`](DECISIONES.md))

Las 9 épicas (E0–E8) están implementadas. Lo que queda no es arquitectura:

1. **Empaquetado** (§1): plataformas objetivo, updater, firma/notarización, iconos de marca.
2. **Pulido de UI** (§2): rails redimensionables por arrastre, overlay de grafo, resaltado con la
   semántica del core.
3. **E0-H04/E6-H03** (§4): generar el `.d.ts` desde Rust (ts-rs/specta).
4. **E7** (§3): adoptar `rmcp` oficial + resources cuando un cliente lo exija.
5. **E8** (§9): gate de bench (§11), threat model.
