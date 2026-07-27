# Auditoría de paridad prototipo ↔ Rust (E1-H18)

> Registro de la auditoría **función-por-función** del core Rust contra el prototipo de referencia
> (`prototype/index.html`), su verificación adversarial y las correcciones aplicadas. El prototipo
> es la **spec de comportamiento** (`CLAUDE.md`): el core debe reproducirlo 1:1, quirks incluidos.

## Método

7 grupos de funciones auditados en paralelo (frontmatter/modelo · links · conformidad · query ·
diff · generadores · grafo), cada divergencia **verificada adversarialmente** (trazando el input
por ambas implementaciones) antes de aceptarla. Resultado: **8 divergencias confirmadas** + 2
falsos positivos refutados (validación de fechas `isISO`: ambos lados son laxos vía `Date.parse`,
no divergen).

> **RETIRADO en `E15-H04`.** Este documento es el registro histórico de la auditoría de paridad
> JS-vs-Rust de v0.2.x. El arnés diferencial y el prototipo como spec de comportamiento se
> retiraron con la migración a workspaces Markdown universales (`ARCHITECTURE.md §20.13`); las
> 6 divergencias que documenta siguen siendo historia útil, no contrato vigente.

El **arnés diferencial** (`prototype/harness/` + `crates/lodestar-core/tests/differential.rs`)
ejecuta las funciones puras del prototipo en Node y compara su salida con el core Rust sobre las
mismas fixtures: es la red de seguridad permanente que fija esta paridad.

## Divergencias corregidas (el core ahora iguala al prototipo)

| # | Función | Divergencia | Corrección |
|---|---|---|---|
| A | `dump_frontmatter` (`model.rs`) | Las claves de productor (extra) se reordenaban alfabéticamente (`BTreeMap`) en vez de conservar el orden de aparición de `Object.keys(fm)`. | `Frontmatter.extra` pasa a `IndexMap` (orden de inserción). `apply_patch` usa `shift_remove`. |
| B | `yaml_is_empty` (`model.rs`) | Un valor `null` se descartaba al serializar; `buildRaw` solo filtra `undefined`/`""`/lista vacía. | `null` ya **no** cuenta como vacío (se emite `clave: null`). Alcanzable en claves extra; los campos-conocidos-null colapsan a ausente por serde (divergencia menor documentada abajo). |
| C | `fm_present` (`query.rs`) | `has:campo` daba `false` para un campo presente con valor `null`. | `null` cuenta como **presente** (solo `undefined`/`""`/lista vacía son ausencia). |
| D | `slugify_tag` (`generate.rs`) | Faltaba la normalización NFC; `café` descompuesto perdía la `é`. | `s.nfc()` antes de los reemplazos (igual que `.normalize("NFC")`). |
| E | `gen_tag_indexes` (`generate.rs`) | Los items de cada índice de tag se ordenaban léxicamente; el prototipo usa `sortPaths` (`localeCompare` numeric). `doc-10` salía antes que `doc-2`. | Nuevo `model::sort_paths_cmp` (orden natural numeric-aware). Solo aplica a items de tags; `genIndex` usa `.sort()` plano y se mantiene. |
| F | `graph_model`/`neighborhood` (`graph.rs`) | Se incluían aristas/nodos a ficheros reservados (`index.md`/`log.md`); el prototipo los descarta (`buildGraphModel:1850`). | Se omiten los targets `is_reserved()` al construir aristas y ghosts. |

Cada fix tiene su test de regresión en `crates/lodestar-core/tests/core.rs`
(sección «E1-H18: paridad con el prototipo»).

## Divergencias intencionales (adiciones ratificadas — NO son bugs)

| Función | Diferencia | Por qué se conserva |
|---|---|---|
| `validate_file` `OKF-CONFLICT` | El core detecta marcadores de merge y hard-failea; el prototipo no tiene ese check. | Adición ratificada (`§10` fila 17). El arnés diferencial **excluye** `OKF-CONFLICT` antes de comparar. |
| `line_diff` guard `MAX_LCS_CELLS` | Para > ~2M celdas el core cae a un diff grueso (todo borra+inserta); el prototipo siempre hace LCS completo. | Cota de memoria/rendimiento (`§11`/`§21`). El objetivo ratificado es Hirschberg (LCS exacto en espacio lineal); pendiente. Solo afecta a ficheros > ~1414 líneas (raro en KB). El arnés usa fixtures por debajo del umbral. |

## Divergencia menor aceptada

Un campo **conocido** (`tags:`/`timestamp:`/…) con valor `null` explícito en el `.md` deserializa a
`None` en serde (no `Some(null)`), así que al reserializar se **omite** en vez de emitir `clave: null`
como el prototipo. Es un caso degenerado (KB con `tags:` vacío); la conformidad coincide en ambos
lados (no se dispara `FMT-TAGS`). Las claves **extra** con `null` sí se preservan (fix B).

## Paridad de la cache (E3, `§5`)

`lodestar-store` materializa columnas + `links`/`tags`/`diagnostics` (checks **locales**) y
**sintetiza** vía SQL backlinks/orphans/dangling/`in_index`. El test obligatorio
(`crates/lodestar-store/tests/store.rs::paridad_*`) verifica que esas proyecciones **igualan**
`lodestar_core::Bundle::from_files(...).analyze()` sobre la misma fixture (conforme, con-issues,
sintético, a-medida y cold-rebuild desde disco). Clave: como los únicos checks `err`/`warn` son
locales, `hard_fail`/`warn_count` se derivan de `diagnostics`; el grafo se sintetiza desde `links`.
Cuando podrían discrepar, **gana el core** (la cache es desechable).
