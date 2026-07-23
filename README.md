# lodestar

**Motor headless de integridad semántica** para bases de conocimiento en formato **OKF** (Open
Knowledge Format): un directorio de ficheros `.md` con frontmatter YAML. «Solo ficheros»: legible
por humanos y por agentes, versionable en git, sin SDK ni servidor. No es un editor generalista y
no gestiona git — es una capa fiable para que agentes (Claude Code, Codex, otros clientes MCP) y
la CLI busquen, validen y analicen conocimiento sin GUI (`ARCHITECTURE.md §19`, giro ratificado
2026-07-22).

Una misma lógica de análisis (conformidad, backlinks, huérfanos, query, grafo) se expone hoy por
**dos fachadas headless**: **CLI** (puerta de CI) y servidor **MCP** (para agentes como Claude
Code) — ambas invocan la misma lógica de `lodestar-core`/`lodestar-workspace` y, desde `E10`, el
crate de servicios `lodestar-app` (`ARCHITECTURE.md §19.2`). La app de escritorio (Tauri v2 +
Svelte 5) sigue en el repo, compilando, pero queda **congelada**: no recibe desarrollo nuevo tras
el giro.

**git sale de la superficie de producto**: ninguna fachada expone ya commit/rama/push/pull/merge.
La mecánica se conserva **dormida** en el crate `lodestar-vcs` (compila, tests verdes, sin
consumidor) por si vuelve a exponerse — ver `ARCHITECTURE.md §19.1` y `§13` (cabecera de
supersesión).

Stack: **Rust + SQLite/FTS5 + MCP + CLI (clap)**; `Tauri v2 + Svelte 5/Vite` para la app de
escritorio congelada; git (libgit2 + binario `git`) como capacidad dormida en `lodestar-vcs`.

## Características

- **Los `.md` en disco son la única fuente de verdad** — todo lo demás (cache SQLite/FTS5, índices,
  grafo) se deriva y se puede reconstruir.
- **Conformidad OKF**: 15+ checks con severidad, salida humana, `--json` y `--sarif`;
  `lodestar check` como puerta de CI con exit codes congelados, sobre el working tree.
- **Motor headless**: la superficie de producto (MCP · CLI) no expone git ni GUI — buscar, validar
  y analizar conocimiento desde agentes y automatización. git queda como capacidad **dormida** en
  `lodestar-vcs` (libgit2 local + binario `git` para red), no en la superficie.
- **Convergencia multi-escritor**: CLI, MCP y edición externa convergen vía un watcher con gate por
  hash blake3; un único escritor aplica cambios (escritura atómica).
- **Escritorio (congelado)**: la app Tauri/Svelte construida antes del giro sigue disponible —
  árbol filtrable, editor con validación y diagnósticos en español, panel de enlaces, grafo
  interactivo (SVG + rAF) — pero no recibe desarrollo nuevo (`ARCHITECTURE.md §19.1`).
- **Paridad garantizada por tests**: la síntesis SQL se verifica idéntica al core, y un arnés
  diferencial ejecuta el prototipo JS original en node como oráculo del core Rust.

## Instalación

### Descargar la app de escritorio (congelada)

> La app de escritorio quedó **congelada** tras el giro a motor headless (`ARCHITECTURE.md §19.1`):
> sigue disponible y funcional, pero no recibe desarrollo nuevo. El foco de producto es la CLI y el
> servidor MCP.

Las builds de escritorio se publican en **[GitHub Releases][releases]** (macOS, Windows y Linux).
Descarga el instalador de tu plataforma desde la última release.

> **Nota — bundles sin firmar.** Los instaladores **no están firmados** todavía, así que el sistema
> operativo mostrará un aviso la primera vez. Es esperado; solo hace falta desbloquearlos una vez.

**macOS** — al abrir `lodestar.app` puede aparecer «no se puede comprobar que no contiene
malware». Dos opciones:

- Clic derecho sobre la app → **Abrir** → **Abrir** en el diálogo (solo la primera vez), o
- quita la cuarentena desde la terminal:

  ```bash
  xattr -dr com.apple.quarantine /ruta/a/lodestar.app
  ```

**Windows** — SmartScreen puede mostrar «Windows protegió su PC». Pulsa **Más información** →
**Ejecutar de todas formas**.

**Linux** — usa el `.AppImage` (dale permiso de ejecución: `chmod +x lodestar_*.AppImage`) o el
paquete `.deb`. Necesitas las libs de WebKitGTK del sistema (ver [Requisitos](#requisitos)).

### Instalar la CLI / el servidor MCP con cargo

La CLI (`lodestar`) y el servidor MCP (`lodestar-mcp`) se pueden compilar e instalar desde el
código con `cargo`:

```bash
cargo install --path crates/lodestar-cli    # binario `lodestar`
cargo install --path crates/lodestar-mcp    # binario `lodestar-mcp`
```

## Requisitos

- **Rust** estable (≥ 1.80, con `rustfmt` y `clippy`; ver `rust-toolchain.toml`)

(Desde `E15-H01`/`E15-H04` no hacen falta node ni git: el arnés diferencial y el crate `vcs` se
retiraron del repo.)

(La UI de escritorio y sus dependencias de sistema Tauri se retiraron de `main`; viven en la rama
`experimental/ui-desktop`.)

## Build desde el código

### Tests
```bash
cargo test --workspace              # core, store, workspace, app, cli, mcp
```

### CLI
```bash
cargo run -p lodestar-cli -- check --path mi-bundle  # ¿conforme? exit 0/1 (--json | --sarif)
cargo run -p lodestar-cli -- reindex                 # reconstruye la cache .lodestar/index.db
```
Subcomandos: `check` · `reindex`. `E15-H02`/`E15-H03` retiraron `init`, `index`, `tags`, `export` e
`import` (no hay ceremonia de creación, ni ficheros con semántica de catálogo, ni formato propio de
intercambio). Desde `E9-H02` tampoco hay subcomandos git ni `--staged`/`--rev`/`--range` en `check`
— juzga siempre el working tree; `E15-H01` borró además el crate `lodestar-vcs`.

Exit codes de `check`: `0` conforme · `1` hard-fail · `2` uso · `3` runtime/IO (el `4`, drift de
generadores, se retiró con los generadores en `E15-H02`).

### App de escritorio (Tauri v2) — retirada de `main`

La UI de escritorio (`frontend/` Svelte + `src-tauri/`, binario `lodestar-desktop`) se **retiró de
`main`** con el giro headless y vive íntegra en la rama `experimental/ui-desktop`. Quien la quiera
construir, esa rama; este repo es un motor headless (CLI + MCP).

### Servidor MCP (agentes)
```bash
cargo run -p lodestar-mcp -- <ruta-al-bundle>   # JSON-RPC por stdio, 10 tools (sin git desde E9-H01)
```

## Estructura del repo

Mapa del giro headless (`ARCHITECTURE.md §19.2`) — `lodestar-app` **llega en E10**, todavía no
existe; hasta entonces `lodestar-cli`/`lodestar-mcp` llaman a `lodestar-workspace` directamente:

```
crates/
  lodestar-core/        # PURO: modelo, conformidad, links, query, grafo, generación, export, diff
  lodestar-store/       # cache SQLite/FTS5 + watcher notify (derivada, desechable)
  lodestar-vcs/         # DORMIDO: git (libgit2 local + binario git para red); mecánica conservada,
                         # sin consumidor de fachada desde E9-H01/H02; nunca escribe el working tree
  lodestar-workspace/   # glue: compone core+store+vcs; único escritor; bus de eventos
  lodestar-app/         # (E10, aún no existe) servicios de caso de uso compartidos por cli/mcp
  lodestar-cli/         # fachada CLI (clap) — sin git en la superficie
  lodestar-mcp/         # fachada MCP (stdio, 10 tools) — sin git en la superficie
  lodestar-fixtures/    # bundles de prueba compartidos (no se publica)
prototype/              # prototipo HTML/JS de referencia + arnés diferencial (oráculo en node)
requirements/           # épicas e historias
```
(La UI de escritorio —`src-tauri/` + `frontend/`— se retiró de `main` a la rama
`experimental/ui-desktop`.)

Los seis crates de la biblioteca (`lodestar-core`, `-store`, `-vcs`, `-workspace`, `-cli`, `-mcp`)
son publicables; `lodestar-fixtures` (solo tests) lleva `publish = false`.

## Documentación

| Documento | Qué es |
|---|---|
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | El diseño ratificado — la autoridad en cuestiones de diseño |
| [`IMPLEMENTATION_STATUS.md`](IMPLEMENTATION_STATUS.md) | Estado real por épica e invariantes verificados |
| [`DECISIONES.md`](DECISIONES.md) | Decisiones de producto aún abiertas, con recomendación |
| [`CHANGELOG.md`](CHANGELOG.md) | Historial de cambios por versión |
| [`RELEASING.md`](RELEASING.md) | Cómo se corta y publica una release |
| [`CLAUDE.md`](CLAUDE.md) | Guía para trabajar en el repo con Claude Code |

## Licencia

Distribuido bajo **MIT OR Apache-2.0**, a tu elección. Ver [`LICENSE-MIT`](LICENSE-MIT) y
[`LICENSE-APACHE`](LICENSE-APACHE).

Salvo que se indique lo contrario, toda contribución que envíes intencionadamente para su
inclusión en la obra, según la licencia Apache-2.0, se licenciará como arriba, sin términos ni
condiciones adicionales.

[releases]: https://github.com/dbareagimeno/lodestar/releases
