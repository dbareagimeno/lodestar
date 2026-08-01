# docs/ — mapa por audiencias

El criterio de este directorio es **vigente / superseded**, no viejo/nuevo
(`ARCHITECTURE.md §21.3`, `DECISIONES.md §17`-DC).

## Para usuarios (inglés)

- [`user/`](user/) — documentación de uso, en inglés. Empieza por el
  quickstart; la demo guiada vive en
  [`../examples/demo/`](../examples/demo/README.md).
  - [`user/quickstart.md`](user/quickstart.md) — instalar, primer `check`,
    cómo leer la salida (severidades y exit codes) y qué leer después.
  - [`user/mcp-clients.md`](user/mcp-clients.md) — configurar un cliente MCP
    (Claude Code y JSON genérico), `--root`, perfiles `readonly`/`standard` y
    recorrido de las 10 tools.
  - [`user/ci.md`](user/ci.md) — `check` como puerta de CI: exit codes
    congelados, `--json`/`--sarif` y un workflow completo de GitHub Actions.
  - Pendientes (E27-H11): la referencia del lenguaje de consulta y la de
    cambios seguros.

## Para el desarrollo del repo (español)

- [`REFACTOR_PHASE_2.md`](REFACTOR_PHASE_2.md) — la **spec de comportamiento
  vigente** de la migración a workspace Markdown universal (junto con
  `ARCHITECTURE.md §20`, que es la autoridad de diseño). No está en
  `history/` porque **gobierna**: ~50 ficheros del repo la citan.
- [`WORKFLOWS.md`](WORKFLOWS.md) — cómo se desarrolla en este repo (SDD, TDD,
  jueces ciegos) y por qué.

## Arqueología (superseded)

- [`history/`](history/) — propuestas y specs que ya no gobiernan, con la
  nota de qué las supersedió. Se conservan como registro de ingeniería, no
  como documentación vigente. El prototipo HTML de v0.2.x
  ([`../prototype/`](../prototype/)) pertenece a la misma categoría.
