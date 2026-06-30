# Estado de implementación

> Mapea las épicas/historias de [`requirements/`](requirements/) a su estado real en esta rama.
> Construido en el **orden de fases ratificado** (`ARCHITECTURE.md §14`), validando con tests en cada fase.
>
> **Resumen**: el backend está sustancialmente implementado y testeado (core + CLI + vcs + workspace +
> MCP), con el frontend y el store/watcher scaffoldeados. **46 tests** en verde; `cargo clippy --workspace
> --all-targets -- -D warnings` limpio; `npm run build`/`check` del frontend en verde.

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
| **E3** `lodestar-store` | 🟡 Scaffold | Crate y superficie reservados. SQLite/FTS5 + watcher `notify` + paridad SQL==core: **pendiente** (es una pieza grande e independiente). La workspace funciona recargando desde disco (el core es la autoridad; la cache es desechable). |
| **E4** `lodestar-vcs` | 🟢 Parcial | libgit2: discover (techo en root)/init/status/RepoState/log/tree_files/commit/branches/conformidad-por-commit/last_conforming; red por binario `git` (pull/push). 8 tests. Pendiente: cache por tree-oid en store, ref-watch, switch/merge target, hooks install, check --staged/--rev. |
| **E5** `lodestar-workspace` | 🟢 Parcial | Handle unificado, único escritor (atómico temp+rename), snapshot, delegaciones, commit con guarda de RepoState + conformidad post-commit, restore con **checkpoint** (no pierde trabajo) + regeneración de index/tags, diff_working, pull/push. 7 tests. Pendiente: bus de eventos en vivo (depende del watcher de E3). |
| **E6** Tauri + frontend | 🟡 Scaffold | Frontend Svelte 5 + Vite compila a `dist/`: variables CSS portadas verbatim, stores (snapshot único + derived de pill/tree), contrato IPC tipado (espejo de Rust), App base. `src-tauri` es placeholder (el cableado de Tauri necesita libs de sistema). Pendiente: tabla de comandos, evento `bundle:changed`, isla del grafo, editor multi-escritor, pill/overlay/modo Cambios. |
| **E7** `lodestar-mcp` | 🟢 Parcial | 13 tools sobre la workspace (backlinks/orphans/dangling/neighborhood/conformance/query/create/update/generate/history/last-conforming/commit) + bucle JSON-RPC por stdio (stdout puro). 1 test e2e. Pendiente: transporte `rmcp` oficial + resources. |
| **E8** Transversales | 🟢 Parcial | Hechos: exit codes/SARIF, escritura atómica, zip-slip cerrado por RelPath, identidad de commits, trailer Co-Authored-By del agente, gitignore de `.lodestar/`. Pendiente: migración del prototipo, packaging/updater, i18n externalizada, lodestar.toml, gate de bench, threat model. |

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

1. **E3** store (SQLite/FTS5 + watcher `notify` + test de paridad) → habilita el bus de eventos en vivo de E5.
2. **E6** port completo de la UI del prototipo + cableado de la fachada Tauri.
3. **E4** cierre: cache de conformidad por tree-oid, switch/merge target, hooks install, check --staged/--rev.
4. **E7** transporte rmcp oficial + golden cross-fachada (E7-H06).
5. **E8** migración, packaging, i18n, lodestar.toml, benches, threat model.
