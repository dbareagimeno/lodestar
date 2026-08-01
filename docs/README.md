# docs/ — mapa por audiencias

El criterio de este directorio es **vigente / superseded**, no viejo/nuevo
(`ARCHITECTURE.md §21.3`, `DECISIONES.md §17`-DC).

## Para usuarios (inglés)

- [`user/`](user/) — documentación de uso: quickstart, clientes MCP, lenguaje
  de consulta, cambios seguros y `check` como puerta de CI. Empieza por
  [`user/quickstart.md`](user/quickstart.md). La demo guiada vive en
  [`../examples/demo/`](../examples/demo/README.md).

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
