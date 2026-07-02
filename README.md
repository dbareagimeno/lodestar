# lodestar

Editor **local-first** de bases de conocimiento en formato **OKF** (Open Knowledge Format): un
directorio de ficheros `.md` con frontmatter YAML. «Solo ficheros»: legible por humanos y por
agentes, versionable en git, sin SDK ni servidor.

Una misma lógica de análisis (conformidad, backlinks, huérfanos, query, grafo) se expone por
**tres fachadas**: app de escritorio (Tauri v2 + Svelte 5), **CLI** (puerta de CI) y servidor
**MCP** (para agentes como Claude Code).

## Características

- **Los `.md` en disco son la única fuente de verdad** — todo lo demás (cache SQLite/FTS5, índices,
  grafo) se deriva y se puede reconstruir.
- **Conformidad OKF**: 15+ checks con severidad, salida humana, `--json` y `--sarif`;
  `lodestar check` como puerta de CI con exit codes congelados.
- **git de primera clase**: commits, ramas locales, merge, push/pull, conformidad-por-commit,
  hooks. Transporte híbrido: libgit2 en local (no ejecuta hooks al abrir = RCE-safe), binario
  `git` solo para la red.
- **Convergencia multi-escritor**: app, CLI, MCP, edición externa y `git pull` convergen vía un
  watcher con gate por hash blake3; un único escritor aplica cambios (escritura atómica).
- **Escritorio**: árbol filtrable, editor con validación y diagnósticos en español, panel de
  enlaces, grafo interactivo (SVG + rAF), modo «Cambios» (diff semántico + commit).
- **Paridad garantizada por tests**: la síntesis SQL se verifica idéntica al core, y un arnés
  diferencial ejecuta el prototipo JS original en node como oráculo del core Rust.

## Estado

Backend y escritorio **completos y testeados** (~91 tests; épicas E0–E8 de
[`IMPLEMENTATION_STATUS.md`](IMPLEMENTATION_STATUS.md)). Pendiente: empaquetado/firma de
instaladores, transporte MCP `rmcp` oficial y generación del `.d.ts` desde Rust — ver
[`DECISIONES.md`](DECISIONES.md). Aún no hay releases binarias: se construye desde el código.

## Requisitos

- **Rust** estable (≥ 1.80, con `rustfmt` y `clippy`; ver `rust-toolchain.toml`)
- **Node.js** ≥ 20 + npm (frontend y arnés diferencial)
- **git** en el PATH (operaciones de red)
- Solo Linux: libs de Tauri (`libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev`, `libgtk-3-dev`,
  `librsvg2-dev`). En macOS/Windows no hace falta nada extra.

## Uso rápido

### Tests
```bash
npm ci --prefix prototype/harness   # deps del arnés diferencial (una vez)
cargo test --workspace              # ~91 tests: core, store, vcs, workspace, cli, mcp + diferenciales
```

### CLI
```bash
cargo run -p lodestar-cli -- init mi-bundle          # bundle nuevo (git init + commit inicial)
cargo run -p lodestar-cli -- check --path mi-bundle  # ¿conforme? exit 0/1 (--json | --sarif)
cargo run -p lodestar-cli -- check --staged          # el árbol staged (ideal como pre-commit)
cargo run -p lodestar-cli -- hooks                   # instala pre-commit → lodestar check
cargo run -p lodestar-cli -- log                     # historial con conformidad por commit
```
Subcomandos: `init` · `check` · `index` · `tags` · `export` · `import` · `reindex` · `log` ·
`last-conforming` · `branch` · `switch` · `merge` · `pull` · `push` · `hooks`.

Exit codes de `check`: `0` conforme · `1` hard-fail · `2` uso · `3` runtime/IO · `4` drift de
generadores.

### App de escritorio (Tauri v2)
```bash
cd frontend && npm ci && npm run dev    # terminal 1: dev server de la webview (:5173)
cargo run -p lodestar-tauri             # terminal 2: la app (binario lodestar-desktop)
```
O sin dev server: `npm run build --prefix frontend && cargo run -p lodestar-tauri --release`.

### Servidor MCP (agentes)
```bash
cargo run -p lodestar-mcp -- <ruta-al-bundle>   # JSON-RPC por stdio, 13 tools
```

## Estructura del repo

```
crates/
  lodestar-core/        # PURO: modelo, conformidad, links, query, grafo, generación, export, diff
  lodestar-store/       # cache SQLite/FTS5 + watcher notify (derivada, desechable)
  lodestar-vcs/         # git: libgit2 local + binario git para red; nunca escribe el working tree
  lodestar-workspace/   # glue: compone core+store+vcs; único escritor; bus de eventos
  lodestar-cli/         # fachada CLI (clap)
  lodestar-mcp/         # fachada MCP (stdio)
  lodestar-fixtures/    # bundles de prueba compartidos
src-tauri/              # fachada de escritorio (Tauri v2)
frontend/               # UI Svelte 5 + Vite (vista fina sobre BundleSnapshot)
prototype/              # prototipo HTML/JS de referencia + arnés diferencial (oráculo en node)
requirements/           # épicas e historias
```

## Documentación

| Documento | Qué es |
|---|---|
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | El diseño ratificado — la autoridad en cuestiones de diseño |
| [`IMPLEMENTATION_STATUS.md`](IMPLEMENTATION_STATUS.md) | Estado real por épica e invariantes verificados |
| [`DECISIONES.md`](DECISIONES.md) | Decisiones de producto aún abiertas, con recomendación |
| [`CLAUDE.md`](CLAUDE.md) | Guía para trabajar en el repo con Claude Code |

## Licencia

MIT OR Apache-2.0
