# E19 — Lenguaje de consulta genérico

> **Fase**: `§20.14` PR 6 (`REFACTOR_PHASE_2 §Fase 5`).
> **Objetivo de la épica**: sustituir la DSL de tokens con semántica de subcadena por un **lenguaje
> de expresiones tipado** sobre cualquier propiedad YAML, con dot-notation, operadores de listas,
> existencia, namespaces y booleanos — y con **un solo AST** al que se traducen tanto la consulta
> textual (`where`) como el filtro estructurado (`filter`). Al cerrar E19, `priority >= 2` funciona,
> `priority >= "high"` es un error de tipo, y `where` y `filter` producen exactamente el mismo
> resultado.
> Referencias maestras: `ARCHITECTURE.md §20.8`, `§20.10`; `docs/REFACTOR_PHASE_2 §Fase 5`;
> `CLAUDE.md` invariantes #3/#4.

**Principio rector**: `§20.8` es explícito y **`§20.2` invariante 4 lo blinda** — *sin coerción
implícita* entre string/número/booleano/lista/objeto. `priority >= "high"` no devuelve `false`: es
un **error de tipo**. Es lo que separa este lenguaje de un grep sofisticado.

**Aviso heredado de E16-H01, en `ARCHITECTURE.md §20.8`**: el evaluador de comparaciones va
**siempre** sobre `ParsedFrontmatter::get` (que devuelve el `Value` con su tipo), **nunca** sobre
`get_text`. Construirlo sobre `get_text` reintroduciría la coerción a string que E16 retiró, **sin
que ningún test lo note** (para números ISO y fechas el orden lexicográfico suele coincidir).

**Decisión de partida (`DECISIONES.md §12`)**: `serde_yaml` 0.9 no tipa timestamps, así que las
fechas son `String`. Para E19 se **declara explícitamente** que la comparación de fechas es
lexicográfica —correcta para ISO-8601 bien formado— y se reevalúa en E20. No se introduce un tipo
fecha propio en esta épica.

**Herencia de E18**: `ParsedFrontmatter::walk` (el iterador `(FieldPath, &Value)`) y `FieldPath`
(E16-H01) ya existen y son la base del acceso a metadata. E19 los **consume**, no los reinventa.

---

### E19-H01 — El AST y el evaluador tipado

- **Objetivo**: el `Expression` de `§20.8` y un evaluador que respeta los tipos YAML sin coerción.
- **Referencias**: `ARCHITECTURE.md §20.8` · `REFACTOR_PHASE_2 §Fase 5 (Operadores mínimos, Semántica
  de tipos, AST unificado)` · `crates/lodestar-core/src/query.rs` (la DSL vieja, que se retira).
- **Alcance**:
  - `Expression` en `core::types` (invariante #4): `Comparison { field: FieldPath, operator, value }`,
    `Function { name, arguments }`, `And(Vec)`, `Or(Vec)`, `Not(Box)`.
  - `ComparisonOperator`: `= != > >= < <=` (comparación) · `contains starts_with ends_with` (texto) ·
    `contains contains_any contains_all` (listas). `FunctionName`: `has`, `missing`.
  - `QueryValue`: el literal tipado (string/número/booleano/null/lista).
  - **Evaluador tipado** `evaluate(expr, doc, analysis) -> Result<bool, TypeError>`:
    - comparaciones de orden (`> >= < <=`) exigen operandos del **mismo tipo** numérico o ambos
      string (lexicográfico); un cruce string/número es `TypeError` — **no** `false`.
    - `contains`/`contains_any`/`contains_all` exigen que el campo sea **lista**; sobre un escalar,
      `TypeError`. (Excepción: `contains` sobre **string** es subcadena — `§Fase 5` lo lista bajo
      «Texto» y bajo «Listas»; el tipo del campo decide cuál aplica.)
    - `=`/`!=` comparan por valor **e igualdad de tipo**: `priority = "2"` sobre `priority: 2`
      (número) es `false`, no error (igualdad entre tipos distintos es `false`; solo el **orden**
      cruzado es error).
  - Va **siempre** sobre `ParsedFrontmatter::get(&FieldPath)`, nunca sobre `get_text`.
- **Fuera de alcance**: parsear texto (E19-H02); el filtro JSON (E19-H03); los namespaces `document`/
  `graph` (E19-H04).
- **Criterios de aceptación**:
  - Igualdad de string, comparación numérica, booleanos → `eq_string`, `cmp_numerico`, `booleano`.
  - `contains` sobre string y sobre lista, `contains_any`, `contains_all` → `contains_string`,
    `contains_lista`, `contains_any_ok`, `contains_all_ok`.
  - `has(x)` y `missing(x)` → `has_ok`, `missing_ok`.
  - Un campo inexistente en una comparación → `false`, no error → `campo_inexistente`.
  - `priority >= "high"` sobre `priority: 2` → `TypeError` → `error_de_tipo_orden_cruzado`.
  - `contains` sobre un escalar → `TypeError` → `error_de_tipo_contains_escalar`.
  - Una propiedad con tipos distintos en dos documentos: cada evaluación respeta el tipo de **su**
    documento → `tipos_heterogeneos`.
- **Dependencias**: E18 completa.
- **Pruebas**: `crates/lodestar-core/tests/`.
- **Frontera (mcp.yml)**: **sí** (`Expression` es contrato de wire para el filtro JSON).

### E19-H02 — El parser textual (`where`)

- **Objetivo**: traducir la consulta textual de `§20.8` al `Expression` de H01.
- **Referencias**: `ARCHITECTURE.md §20.8` · `REFACTOR_PHASE_2 §Fase 5 (Consultas básicas,
  Expresiones booleanas, Existencia, Namespaces)`.
- **Alcance**:
  - Parser de `type = "decision" and (status = "draft" or status = "review") and not tags contains
    "archived"`: literales entrecomillados (string), sin comillas (número/booleano/null por su
    forma), dot-notation (`service.tier`), `has(x)`/`missing(x)`, `and`/`or`/`not`, paréntesis y
    **precedencia** (`not` > `and` > `or`).
  - **Abreviatura de namespace**: `status = "x"` ≡ `frontmatter.status = "x"`. Las propiedades
    calculadas (`document.*`, `graph.*`) **exigen** namespace explícito (su semántica es E19-H04;
    aquí basta con que el parser las reconozca sintácticamente y produzca el `FieldPath`
    namespaced).
  - Errores de parseo son `Result`, no panics ni queries vacías (E20/E21 los mapearán a
    `INVALID_SCHEMA`).
- **Fuera de alcance**: evaluar `document.*`/`graph.*` (E19-H04); la equivalencia con el filtro JSON
  (E19-H03).
- **Criterios de aceptación**:
  - `and`, `or`, `not`, paréntesis, precedencia → `and_ok`, `or_ok`, `not_ok`, `parentesis`,
    `precedencia`.
  - `service.tier = "critical"` (dot notation) → `dot_notation_textual`.
  - `status = "accepted"` produce el mismo AST que `frontmatter.status = "accepted"` →
    `abreviatura_de_namespace`.
  - Un número sin comillas es número, un booleano es booleano, `"2"` es string →
    `literales_por_forma`.
  - Una consulta malformada (`status =`) → `Err`, no panic → `parseo_malformado_es_error`.
- **Dependencias**: E19-H01.
- **Pruebas**: `crates/lodestar-core/tests/`.
- **Frontera (mcp.yml)**: no (el `where` es un string; el AST ya está en el contrato por H01).

### E19-H03 — El filtro JSON y la equivalencia

- **Objetivo**: el `filter` estructurado de `§20.10` traduce al **mismo** `Expression`, y `where` y
  `filter` producen **exactamente el mismo resultado** (`§Fase 5`, «AST unificado»).
- **Referencias**: `ARCHITECTURE.md §20.8`, `§20.10` · `REFACTOR_PHASE_2 §Fase 5 (Superficie MCP)`.
- **Alcance**:
  - Deserialización del filtro JSON de `§20.10` (`{and: [{field, operator, value}, …]}`) a
    `Expression`. `operator` en el wire usa nombres largos (`equals`, `contains`), mapeados a
    `ComparisonOperator`.
  - **Test de equivalencia**: para un conjunto de consultas, `parse(where)` y
    `from_json(filter)` producen `Expression`s iguales **y** el mismo conjunto de documentos.
- **Fuera de alcance**: cablear esto a `knowledge_search` (E19-H05).
- **Criterios de aceptación**:
  - Un filtro JSON con `and`/comparación/lista deserializa al `Expression` correcto →
    `filtro_json_deserializa`.
  - Para 6+ consultas de `§Fase 5`, `where` y `filter` dan el **mismo AST** → `equivalencia_ast`.
  - …y el **mismo conjunto de documentos** sobre un workspace → `equivalencia_resultado`.
- **Dependencias**: E19-H01, H02.
- **Pruebas**: `crates/lodestar-core/tests/` (equivalencia de AST) y `crates/lodestar-app/tests/`
  (equivalencia de resultado).
- **Frontera (mcp.yml)**: **sí** (la forma del `filter`).

### E19-H04 — Namespaces calculados (`document.*`, `graph.*`)

- **Objetivo**: consultar propiedades del documento y del grafo, no solo del frontmatter.
- **Referencias**: `ARCHITECTURE.md §20.8` (Namespaces) · `REFACTOR_PHASE_2 §Fase 5 (Namespaces,
  Ejemplos)`.
- **Alcance**:
  - `document.path` · `document.title` · `document.has_frontmatter`.
  - `graph.backlinks` (nº) · `graph.outgoing_links` (nº) · `graph.dangling_links` (nº) ·
    `graph.isolated` (bool).
  - El evaluador resuelve estos campos desde `Analysis`/el documento en vez de desde el frontmatter.
    **Exigen namespace explícito** (`§20.8`): `isolated = true` **no** es `graph.isolated = true` —
    el primero busca una clave de frontmatter `isolated`.
- **Fuera de alcance**: `metadata_inspect` (E20).
- **Criterios de aceptación**:
  - `document.path starts_with "docs/"` → `namespace_document_path`.
  - `document.has_frontmatter = false` selecciona los documentos sin frontmatter →
    `namespace_has_frontmatter`.
  - `graph.backlinks = 0` selecciona los no enlazados; `graph.dangling_links > 0` los que tienen
    enlaces rotos → `namespace_graph_backlinks`, `namespace_graph_dangling`.
  - `graph.isolated = true` selecciona los aislados; una clave de frontmatter `isolated` **no**
    interfiere → `namespace_graph_isolated`.
- **Dependencias**: E19-H01, H02.
- **Pruebas**: `crates/lodestar-core/tests/`.
- **Frontera (mcp.yml)**: no (nombres de campo dentro del `where`/`filter`).

### E19-H05 — Cablear el lenguaje a `knowledge_search`

- **Objetivo**: que `knowledge_search` acepte `where`/`filter` y que `SearchFilters`/`SearchResult`
  dejen de hablar de `type`/`status`/`tags`.
- **Referencias**: `ARCHITECTURE.md §20.10` · `crates/lodestar-app/src/lib.rs` (`knowledge_search`,
  `SearchFilters`, `SearchResult`, `passes_filters`), `crates/lodestar-mcp/src/tools.rs`.
- **Alcance**:
  - `knowledge_search` acepta `where` (string) y `filter` (JSON), ambos → `Expression` → filtro.
    Se combinan con el FTS (`text`) por intersección, como hoy.
  - **Retirar `SearchFilters`** con sus campos OKF `types`/`statuses`/`tags`; el filtrado por
    metadata pasa por el lenguaje. `SearchResult` pierde `type`/`status`/`description`/`tags` como
    campos privilegiados — conserva `path`, `title` (derivado) y el snippet.
  - Retirar `DocumentSet::query`/`tokenize_query`/`match_token` (la DSL vieja) y `query.rs` entero.
  - Actualizar `contracts/mcp.yml`: `knowledge_search` con `where`/`filter`, `SearchResult` nuevo.
- **Fuera de alcance**: las selecciones masivas por consulta que alimentan `change_plan` (E21).
- **Criterios de aceptación**:
  - `knowledge_search {where: "status = \"accepted\""}` filtra por esa metadata →
    `search_where`.
  - `knowledge_search {filter: {…}}` equivalente da el mismo resultado → `search_filter_equivalente`.
  - `knowledge_search {where: "graph.backlinks = 0"}` devuelve los no enlazados →
    `search_propiedad_de_grafo`.
  - El resultado no lleva `type`/`status`/`tags` privilegiados → `search_result_sin_campos_okf`.
- **Decisión tomada en la fase verde**: cuando una expresión **bien formada** produce un `TypeError`
  al evaluarse contra un **documento concreto** (p. ej. `priority >= 2` sobre un documento cuyo
  `priority` es `"alto"`, un string), ese documento se **excluye** del resultado, sin abortar la
  búsqueda. Sobre un corpus heterogéneo, que un solo documento con un tipo incompatible tumbe la
  consulta sobre los demás sería frágil y dependiente del orden. (Distinto de un `where`/`filter`
  **malformado** —no parseable—, que sí es un error de la consulta entera; su mapeo a
  `INVALID_SCHEMA` es E20.)
- **Dependencias**: E19-H01…H04.
- **Pruebas**: `crates/lodestar-mcp/tests/`, `crates/lodestar-app/tests/`.
- **Frontera (mcp.yml)**: **sí**.

---

## Orden de construcción

```
H01 (AST + evaluador) ─→ H02 (parser textual) ─→ H03 (filtro JSON + equivalencia) ─→ H05 (cableado)
                              └─→ H04 (namespaces) ──────────────────────────────────┘
```

## Criterio de salida

`priority >= "high"` es un error de tipo (no `false`); `where` y `filter` dan el mismo resultado
sobre cualquier workspace; se puede consultar cualquier propiedad YAML anidada y las propiedades
calculadas del documento y del grafo; y `grep -rn "tokenize_query\|SearchFilters" crates` no
encuentra la DSL vieja ni los filtros OKF.
