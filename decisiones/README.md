# Decisiones

Esta carpeta recoge las decisiones que **no se pueden tomar por inercia** desde el código o
`ARCHITECTURE.md` y que dependen de tu criterio de producto/entorno. Cada decisión es un fichero
`.md` con frontmatter YAML; el número `§N` sigue siendo el identificador estable de cada una, y lo
que antes vivía en el encabezado del documento único (estado, prioridad, revisión) vive ahora en
ese frontmatter, consultable por el propio motor. Nada aquí bloquea lo ya implementado (el motor
headless E9–E27 está completo y testeado); son decisiones para cerrar el último tramo del proyecto
y para afinar comportamiento.

## Cómo consultarlas

El frontmatter de cada fichero es consultable con el propio motor Lodestar: `knowledge_search` con
`where` (y `metadata_inspect` para el catálogo de propiedades). Algunos ejemplos:

```
prioridad >= 4 and estado = "abierta"
# lo que exige criterio hoy → devuelve 5: §14, §15, §16, §18, §19

estado = "abierta"
# todo lo vivo

etiquetas contains "contrato"
# por tema

revisada_en < "2026-07-01" and estado = "abierta"
# decisiones abiertas que nadie toca

reabrible = true
# cerradas con una mitad que puede volver (§17)
```

La prioridad va de **1 a 5, donde 5 es lo más importante**. El lenguaje no tiene `sort`: la
prioridad **filtra**, no ordena. El vocabulario de `estado` es cerrado: `abierta`, `tomada`,
`ratificada`, `cerrada`, `diferida`, `obsoleta`.

## Índice

| § | Decisión | Estado | Prio | Doc |
|---|---|---|---|---|
| 0 | Giro a motor headless de integridad semántica | ratificada | 2 | [`00-giro-headless.md`](00-giro-headless.md) |
| 1 | Build de la fachada de escritorio Tauri | diferida | 2 | [`01-build-fachada-escritorio.md`](01-build-fachada-escritorio.md) |
| 2 | Port de la UI del prototipo | obsoleta | 1 | [`02-port-ui-prototipo.md`](02-port-ui-prototipo.md) |
| 3 | Transporte MCP: stdio propio frente a rmcp | abierta | 2 | [`03-transporte-mcp-rmcp.md`](03-transporte-mcp-rmcp.md) |
| 4 | Generación del `.d.ts` desde Rust | obsoleta | 1 | [`04-generacion-dts-ts-rs.md`](04-generacion-dts-ts-rs.md) |
| 5 | i18n multi-idioma | abierta | 1 | [`05-i18n-multi-idioma.md`](05-i18n-multi-idioma.md) |
| 6 | Semántica de `merge` local | obsoleta | 1 | [`06-semantica-merge-local.md`](06-semantica-merge-local.md) |
| 7 | `lodestar check --range a..b` | obsoleta | 1 | [`07-check-range.md`](07-check-range.md) |
| 8 | Esquema de `lodestar.toml` | obsoleta | 1 | [`08-esquema-lodestar-toml.md`](08-esquema-lodestar-toml.md) |
| 9 | Transversales diferidas de producto | abierta | 2 | [`09-transversales-diferidas.md`](09-transversales-diferidas.md) |
| 10 | Ghosts como primitiva de planificación + templates | abierta | 3 | [`10-ghosts-y-templates.md`](10-ghosts-y-templates.md) |
| 11 | `pulldown-cmark` en `lodestar-core` | tomada | 1 | [`11-pulldown-cmark-en-core.md`](11-pulldown-cmark-en-core.md) |
| 12 | Comparación de fechas en el lenguaje de consulta | cerrada | 1 | [`12-fechas-en-consultas.md`](12-fechas-en-consultas.md) |
| 13 | `Conformant → Valid` | cerrada | 1 | [`13-conformant-a-valid.md`](13-conformant-a-valid.md) |
| 14 | El store (E18) no tiene ningún consumidor | abierta | **5** | [`14-store-sin-consumidor.md`](14-store-sin-consumidor.md) |
| 15 | ¿Rechazar los parámetros no declarados? | abierta | 4 | [`15-parametros-no-declarados.md`](15-parametros-no-declarados.md) |
| 16 | Deuda declarada por la auditoría de E25/E26 | abierta | 4 | [`16-deuda-auditoria-e25-e26.md`](16-deuda-auditoria-e25-e26.md) |
| 17 | Superficie externa y apertura OSS | cerrada | 3 | [`17-superficie-externa-oss.md`](17-superficie-externa-oss.md) |
| 18 | `canApply: false` no vincula a `change_apply` | abierta | 4 | [`18-canapply-no-vincula-apply.md`](18-canapply-no-vincula-apply.md) |
| 19 | Hallazgos de documentar la referencia de usuario | abierta | 4 | [`19-hallazgos-referencia-usuario.md`](19-hallazgos-referencia-usuario.md) |

## Dónde está el criterio hoy

Lo que exige criterio **hoy** son las cinco decisiones de prioridad ≥ 4:

- **§14** — el store (E18) se construyó sin ningún consumidor. Es la que gobierna a las demás:
  mientras siga abierta, `ARCHITECTURE.md §21.5` prohíbe que la superficie externa prometa la cache
  o el rendimiento a escala.
- **§15** — la superficie declara `additionalProperties: false` y no lo ejecuta.
- **§16** — doce deudas registradas por los jueces ciegos de E25/E26, todavía sin resolver.
- **§18** — `canApply: false` es una señal que el `change_apply` no ejerce.
- **§19** — dos discrepancias contrato↔motor detectadas al documentar la referencia de usuario.

**§10** (ghosts como primitiva de planificación + templates) sigue siendo la siguiente feature
acordada, pendiente de puerta de diseño.

**§3, §5, §9** son decisiones vivas de baja urgencia: tienen recomendación estable y esperan un
caso real que las fuerce.

**§1** queda diferida: la firma/notarización de los binarios de CLI/MCP sigue sin resolverse.
**§17** está cerrada pero es reabrible: su mitad de crates.io bloquea `E27-H10`.

**§2, §4, §6, §7, §8** son archivo: la pregunta ya no tiene sujeto porque el código que la motivaba
se borró (la UI de escritorio, el espejo TS, el merge local, `--range`, `lodestar.toml`). Se
conservan por trazabilidad, no por vigencia.
