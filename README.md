# lodestar

**Un motor local y transaccional para que agentes de IA descubran, consulten, comprendan y
modifiquen de forma segura una red arbitraria de documentos Markdown contenida dentro de un
proyecto** (`ARCHITECTURE.md §20`). «Solo ficheros `.md`»: legibles por humanos y por agentes,
versionables en git, sin SDK ni servidor ni formato propio.

```bash
cd mi-proyecto
lodestar-mcp
```

Eso es todo: Lodestar usa el directorio actual como raíz del workspace y descubre recursivamente
todos los `.md`, a cualquier profundidad, respetando `.gitignore` y `.lodestarignore`. **No** hace
falta `lodestar init`, ni `.lodestar/config.yaml`, ni `index.md`, ni frontmatter, ni declarar un
campo `type`. Cualquier estructura de carpetas vale.

Su valor no depende de un formato propio, sino de: descubrimiento global, consultas estructuradas
sobre frontmatter, grafo de documentos y backlinks, análisis de impacto, planificación de cambios,
validación previa, escrituras atómicas, auditoría, recovery y rollback.

Se consume desde **Claude Code, Codex u otros clientes MCP** y desde la **CLI** (puerta de CI), sin
editor, sin GUI y sin git en la superficie.

> **v0.3.0 es incompatible con v0.2.x.** Lodestar dejó de exigir el formato **OKF** propio
> (`ARCHITECTURE.md §19`, hoy superado por `§20`). Un repositorio OKF existente sigue siendo Markdown
> válido y se abre sin migración; usa `lodestar migrate-from-okf --dry-run` para diagnosticar las
> convenciones legadas. Ver [`CHANGELOG.md`](CHANGELOG.md).

## Características

- **Los `.md` en disco son la única fuente de verdad** — todo lo demás (cache SQLite/FTS5, grafo,
  metadata indexada) se deriva y se puede reconstruir.
- **Frontmatter YAML arbitrario**: cualquier clave es válida, con su tipo YAML real; nada es
  obligatorio; ningún nombre de fichero (`index.md`, `README.md`) activa reglas especiales.
- **Enlaces Markdown estándar** resueltos solo por path (inline, de referencia, anchors, externos),
  entre cualquier profundidad; grafo universal con backlinks globales.
- **Lenguaje de consulta tipado**: `status = "accepted" and priority >= 2`, `owners contains
  "security"`, `graph.backlinks = 0` — sobre cualquier propiedad YAML, con dot-notation y **sin
  coerción de tipos** (`priority >= "high"` es un error de tipo, no un `false` silencioso). La
  consulta textual (`where`) y el filtro JSON (`filter`) producen el mismo resultado.
- **Inspección de metadata sin schema**: `metadata_inspect` descubre qué campos usa una base
  desconocida, en cuántos documentos aparece cada uno y qué valores toma.
- **Modelo transaccional recuperable**: `change_plan` (normaliza/simula/valida, `planHash`) →
  `change_apply` (staging → lock → backup → write-ahead journal → renames atómicos → receipt, con
  crash-recovery determinista) → `change_revert`. Un cambio nunca introduce errores nuevos; un repo
  que ya tiene problemas se puede reparar parcialmente.
- **`lodestar check` como puerta de CI** con exit codes congelados, sobre el working tree.

## Instalación

La CLI (`lodestar`) y el servidor MCP (`lodestar-mcp`) se compilan e instalan desde el código con
`cargo`:

```bash
cargo install --path crates/lodestar-cli    # binario `lodestar`
cargo install --path crates/lodestar-mcp    # binario `lodestar-mcp`
```

## Requisitos

- **Rust** estable (≥ 1.80, con `rustfmt` y `clippy`; ver `rust-toolchain.toml`).

No hacen falta node, git ni librerías de sistema: el arnés diferencial, el crate `lodestar-vcs` y la
UI de escritorio se retiraron del repo en la migración a Markdown universal.

## Uso

### Servidor MCP (agentes)

```bash
cargo run -p lodestar-mcp                      # JSON-RPC por stdio, 10 tools; la raíz es el cwd
cargo run -p lodestar-mcp -- --root <dir>      # …o el directorio indicado
cargo run -p lodestar-mcp -- --profile readonly  # solo las tools de lectura/verificación
```

Las **10 tools** de la superficie MCP:

| Tool | Qué hace |
|---|---|
| `workspace_status` | Config, capacidades del perfil, conformidad y recuento agregado (llámala primero). |
| `knowledge_search` | Localiza documentos por texto libre + `where`/`filter` (lenguaje tipado); nunca cuerpos. |
| `knowledge_get` | Un documento con `include` selectivo (frontmatter, body, enlaces, backlinks, diagnósticos). |
| `metadata_inspect` | Catálogo de propiedades del workspace, o inspección de un campo (tipos, valores frecuentes). |
| `graph_query` | Backlinks, salientes, vecindario, aislados, dangling, caminos, ciclos, componentes. |
| `impact_analyze` | Impacto de un `move`/`delete` hipotético: afectados directos y transitivos, riesgo. |
| `knowledge_check` | Audita el workspace (diagnósticos con id estable, severidad configurable). |
| `change_plan` | Planifica un cambio SIN escribir (o una selección masiva por consulta); `planHash`. |
| `change_apply` | Aplica el plan por el único escritor, con todas las salvaguardas transaccionales. |
| `change_revert` | Revierte una transacción reciente al estado anterior desde sus copias de recuperación. |

### CLI (puerta de CI)

```bash
cargo run -p lodestar-cli -- check              # ¿interpretable y consistente? exit 0/1 (--json | --sarif)
cargo run -p lodestar-cli -- reindex            # reconstruye la cache .lodestar/index.db
cargo run -p lodestar-cli -- migrate-from-okf --dry-run   # diagnostica convenciones OKF legadas
```

Subcomandos: `check` · `reindex` · `migrate-from-okf`. Exit codes de `check`: `0` conforme · `1`
hard-fail · `2` uso · `3` runtime/IO.

## Build desde el código

```bash
cargo test --workspace --locked        # la suite completa
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

## Estructura del repo

```
crates/
  lodestar-core/        # PURO: modelo, frontmatter, links (pulldown-cmark), query tipada, grafo,
                         #       metadata, diff. Sin I/O, sin DB, sin runtime.
  lodestar-store/        # cache SQLite/FTS5 + watcher notify (derivada, desechable)
  lodestar-workspace/    # glue: descubrimiento, único escritor, staging/journal/locks/recovery, bus
  lodestar-app/          # servicios de caso de uso compartidos por cli/mcp (envelope, códigos de error)
  lodestar-cli/          # fachada CLI (clap)
  lodestar-mcp/          # fachada MCP (stdio, 10 tools)
  lodestar-fixtures/     # workspaces de prueba compartidos (no se publica)
prototype/               # prototipo HTML/JS de la era OKF — referencia histórica de v0.2.x
requirements/            # épicas e historias
```

## Documentación

| Documento | Qué es |
|---|---|
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | El diseño ratificado (§20 es la autoridad vigente) |
| [`IMPLEMENTATION_STATUS.md`](IMPLEMENTATION_STATUS.md) | Estado real por épica e invariantes verificados |
| [`DECISIONES.md`](DECISIONES.md) | Decisiones de producto aún abiertas, con recomendación |
| [`CHANGELOG.md`](CHANGELOG.md) | Historial de cambios por versión |
| [`CLAUDE.md`](CLAUDE.md) | Guía para trabajar en el repo con Claude Code |

## Licencia

Distribuido bajo **MIT OR Apache-2.0**, a tu elección. Ver [`LICENSE-MIT`](LICENSE-MIT) y
[`LICENSE-APACHE`](LICENSE-APACHE).

Salvo que se indique lo contrario, toda contribución que envíes intencionadamente para su inclusión
en la obra, según la licencia Apache-2.0, se licenciará como arriba, sin términos ni condiciones
adicionales.
