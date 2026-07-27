# E18 — Store v2

> **Fase**: `§20.14` PR 5 (`REFACTOR_PHASE_2 §Fase 9`).
> **Objetivo de la épica**: que la cache deje de ser un espejo del modelo OKF y pase a materializar el
> modelo genérico: **metadata anidada por field path**, **enlaces con su clasificación**, y un FTS que
> no depende de campos concretos. Al cerrar E18, el store se reconstruye desde cero sin un solo dato
> OKF y el test de paridad vuelve a comparar manzanas con manzanas.
> Referencias maestras: `ARCHITECTURE.md §20.12`, `§5`; `CLAUDE.md` invariantes #1/#3/#5.

**Principio rector**: `§20.12` lo dice sin ambigüedad — *"el índice SQLite es derivado y desechable:
se incrementa `USER_VERSION` y se reconstruye por completo, sin migración de datos OKF"*. No hay
compatibilidad hacia atrás que preservar. Y el invariante #3 sigue mandando: **cuando el core y la
cache puedan discrepar, gana el core**; la cache es un acelerador verificado por el test de paridad.

**Estado de partida**: E16/E17 ya movieron el DDL dos veces (fuera `files.kind` y
`links.src_is_index` con bump 2→3). Lo que queda no es un retoque: es el modelo de datos nuevo.

---

### E18-H01 — DDL v2: `documents` y `metadata`

- **Objetivo**: materializar el frontmatter **genérico**, indexado por field path recursivo.
- **Referencias**: `ARCHITECTURE.md §20.12` · `REFACTOR_PHASE_2 §Fase 9` ·
  `crates/lodestar-store/src/schema.rs`, `index.rs`.
- **Alcance**:
  - `files` → `documents(path, title, body, raw, frontmatter_json, content_hash)`. Desaparecen las
    columnas promovidas `type`/`status`/`description`/`resource`: eran los campos conocidos de OKF.
    `title` se conserva porque es el **título derivado** (`§20.4`), no un campo del usuario.
  - Tabla nueva `metadata(document_path, field_path, value_json, value_type)`, poblada
    **recursivamente**: `service: {name, tier}` produce `service.name` y `service.tier`. Conserva el
    valor JSON original **y su tipo** — es lo que `metadata_inspect` (E20) necesita para comunicar la
    heterogeneidad, y lo que E19 necesita para comparar sin coerción.
  - El recorrido reutiliza `FieldPath`/`ParsedFrontmatter::get` de `core` (E16-H01): **una sola
    verdad de acceso a metadata**, nunca un segundo navegador del `Value` en SQL.
  - Retirar la tabla `tags` (era el índice del campo OKF `tags`; ahora es metadata como cualquiera).
  - Bump de `USER_VERSION` y rebuild limpio.
- **Fuera de alcance**: consultar esa metadata (E19); inspeccionarla (E20).
- **Criterios de aceptación**:
  - **Dado** un documento con `service: {name: auth, tier: critical}`, **Cuando** se indexa,
    **Entonces** hay filas `service.name` y `service.tier` con su valor y su tipo →
    `metadata_indexa_paths_anidados`.
  - **Dado** un documento con `priority: 2` y otro con `priority: "alta"`, **Cuando** se indexan,
    **Entonces** las dos filas conservan tipos distintos (`number` y `string`) →
    `metadata_conserva_el_tipo`.
  - **Dado** un documento con listas y objetos anidados en listas, **Cuando** se indexa, **Entonces**
    el `value_json` permite reconstruir el valor original → `metadata_roundtrip_json`.
  - **Dado** una cache de v0.3 con el DDL viejo, **Cuando** se abre, **Entonces** se detecta antigua
    y se reconstruye → `cache_v3_se_reconstruye`.
- **Dependencias**: E17 completa.
- **Pruebas**: `crates/lodestar-store/tests/`.
- **Frontera (mcp.yml)**: no (el DDL es interno; el store es su dueño único).

### E18-H02 — `links` y `diagnostics` genéricos

- **Objetivo**: materializar los enlaces con su clasificación, no solo las aristas resueltas.
- **Referencias**: `ARCHITECTURE.md §20.12` · `crates/lodestar-store/src/index.rs`, `synth.rs`.
- **Alcance**:
  - `links(source_path, raw_href, target_kind, target_path, fragment, resolved)` — la forma de
    `§20.12`. Hoy solo se materializan las aristas internas resueltas, así que la cache no puede
    responder por externos, anchors ni `WorkspaceFile`.
  - `diagnostics(document_path, code, severity, message, range_json)` — gana `range_json`, que el
    catálogo de `§20.9` ya produce (`FM-YAML-INVALID` lo rellena desde E16-H05).
  - **Cerrar la asimetría declarada en E17**: la cache resuelve con `Inventory::default()`, así que
    su filtro de aristas es puramente por extensión y pierde los `WorkspaceFile`. Con `target_kind`
    materializado, la cache debe reflejar la misma clasificación que el core.
- **Fuera de alcance**: los diagnósticos de **descubrimiento** (ver E18-H04).
- **Criterios de aceptación**:
  - **Dado** un documento con enlaces de los 5 tipos, **Cuando** se indexa, **Entonces** cada uno
    tiene su `target_kind` y los externos/anchors conservan su `raw_href` →
    `links_materializa_las_5_clases`.
  - **Dado** un enlace con fragmento, **Cuando** se indexa, **Entonces** `fragment` está poblado y
    `target_path` no lo incluye → `links_separa_el_fragmento`.
  - **Dado** un `FM-YAML-INVALID`, **Cuando** se indexa, **Entonces** su `range_json` sobrevive al
    round-trip → `diagnostics_conserva_el_rango`.
- **Dependencias**: E18-H01.
- **Pruebas**: `crates/lodestar-store/tests/`.
- **Frontera (mcp.yml)**: no.

### E18-H03 — FTS sin campos privilegiados

- **Objetivo**: el índice de texto deja de depender de `type`/`status`/`tags`.
- **Referencias**: `ARCHITECTURE.md §20.12` (*"indexar path, título derivado, body y valores
  textuales de frontmatter; no depender de campos concretos"*) · `crates/lodestar-store/src/lib.rs`.
- **Alcance**: `documents_fts` sobre `(path, title, body, frontmatter_text)`, donde
  `frontmatter_text` es la concatenación de los **valores textuales** de la metadata del documento
  (los escalares string; los números y booleanos no aportan al texto libre).
- **Criterios de aceptación**:
  - **Dado** un documento con `owners: [platform, security]`, **Cuando** se busca «security»,
    **Entonces** aparece → `fts_encuentra_valores_de_frontmatter`.
  - **Dado** dos documentos con la misma palabra en el body y en el frontmatter, **Cuando** se
    buscan, **Entonces** ambos aparecen → `fts_no_privilegia_campos`.
- **Dependencias**: E18-H01.
- **Pruebas**: `crates/lodestar-store/tests/`.
- **Frontera (mcp.yml)**: no.

### E18-H04 — Paridad core ↔ store bajo el modelo nuevo

- **Objetivo**: restaurar la garantía del invariante #3 sobre el modelo genérico.
- **Referencias**: `ARCHITECTURE.md §20.12` (*"el resultado calculado por el core puro debe coincidir
  con el recuperado desde SQLite"*), `§10` fila 1 · `crates/lodestar-store/tests/store.rs`
  (`assert_matches_core`, `property_incremental_igual_core`).
- **Alcance**:
  - `assert_matches_core` compara la `Analysis` completa del modelo nuevo: `documents`, `outgoing`
    con su clasificación, `incoming`, `isolated`, `dangling` y `diagnostics`.
  - **Cerrar la asimetría del inventario**: el store debe construir su `Inventory` con los mismos
    `other_files` que el core, o declarar por qué no puede y qué se pierde.
  - La property test incremental (120 ediciones) sigue valiendo sobre el modelo nuevo.
- **Criterios de aceptación**:
  - **Dado** el fixture `with_edge_cases()` (5 clases de enlace, mismo basename, capitalización),
    **Cuando** se compara core vs store, **Entonces** las dos `Analysis` son idénticas →
    `paridad_con_edge_cases`.
  - **Dado** 120 ediciones aleatorias deterministas, **Cuando** se aplican incrementalmente,
    **Entonces** el store coincide con un rebuild completo → `property_incremental_igual_core`.
- **Dependencias**: E18-H01, H02, H03.
- **Pruebas**: `crates/lodestar-store/tests/store.rs`.
- **Frontera (mcp.yml)**: no.

---

## Orden de construcción

```
H01 (documents + metadata) ─→ H02 (links + diagnostics) ─→ H04 (paridad)
        └─→ H03 (FTS) ─────────────────────────────────────┘
```

## Criterio de salida

`grep -rn "type\|status\|tags" crates/lodestar-store/src/schema.rs` no encuentra ninguna columna
promovida de OKF; la cache se reconstruye desde cero sobre un workspace arbitrario; y el test de
paridad compara la `Analysis` completa del modelo nuevo, incluida la clasificación de enlaces.
