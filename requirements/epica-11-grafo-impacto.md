# E11 — Grafo e impacto

> **Fase**: `§19.8` fase 2 (`REFACTOR §16`). **Objetivo de la épica**: que Lodestar responda preguntas
> **estructurales** y **anticipe consecuencias** de cambios. Consolida las cuatro tools de grafo actuales
> en **`graph_query`** (con operaciones nuevas: `path_between`/`cycles`/`components`), añade **relaciones
> tipadas** validadas contra el schema y la **validación de paths externos** (`referenceRoots`), y entrega
> **`impact_analyze`** (reusa el blast-radius del store).
> Criterio de salida (`REFACTOR §16`): *Lodestar responde preguntas estructurales y anticipa consecuencias
> de cambios*.
> Referencias maestras: `ARCHITECTURE.md §19.6`, `§19.2` · `REFACTOR §9.5, §9.6, §16 fase 2` ·
> `core::graph` (`graph.rs`), `Store::blast_radius` (`store/lib.rs:296`, `synth.rs:110`), `Bundle::neighborhood`.

**Principio rector de la épica**: *el grafo es una verdad computada del core; SQLite acelera, no decide*
(invariantes #3). Las operaciones nuevas se implementan como funciones puras del core y, cuando hay una
proyección SQL equivalente (blast-radius), se verifica idéntica por paridad.

---

### E11-H01 — `graph_query` (consolida backlinks/outgoing/neighborhood/orphans/dangling)
- **Objetivo**: una sola tool de grafo con las operaciones que hoy son 4 tools separadas.
- **Referencias**: `ARCHITECTURE.md §19.6` · `REFACTOR §9.5, §15` · `Bundle::backlinks`/`neighborhood`,
  `Analysis.orphans`/`dangling`.
- **Alcance**:
  - Servicio `App::graph_query(operation, ref?, depth?, direction?, limit?, cursor?)` con operaciones
    `backlinks`/`outgoing`/`neighborhood`/`orphans`/`dangling` → `{ nodes, edges, summary{nodeCount,
    edgeCount,truncated}, nextCursor }`.
  - Tool MCP `graph_query` con `inputSchema`/`outputSchema`.
  - Mapea las tools retiradas: `find_backlinks`→`operation:backlinks`, etc.
- **Fuera de alcance**: `path_between`/`cycles`/`components` (E11-H02); impacto (E11-H05).
- **Criterios de aceptación**:
  - **Dado** un concepto con 3 backlinks, **Cuando** se llama `graph_query(operation:backlinks)`,
    **Entonces** los 3 aparecen en `nodes`/`edges` → `graph_backlinks`.
  - **Dado** `operation:neighborhood, depth:2, direction:both`, **Cuando** se llama, **Entonces** el
    subgrafo casa con `Bundle::neighborhood` del core → `graph_neighborhood_paridad`.
  - **Dado** `operation:orphans`, **Cuando** se llama, **Entonces** lista los conceptos sin entrantes →
    `graph_orphans`.
  - **Dado** `limit` menor que los nodos, **Cuando** se llama, **Entonces** `summary.truncated == true` y
    `nextCursor` presente → `graph_truncado`.
- **Dependencias**: E10-H01, E10-H03, E10-H04.
- **Pruebas**: `crates/lodestar-mcp/tests/`: `graph_backlinks`, `graph_neighborhood_paridad`, `graph_orphans`,
  `graph_truncado`.
- **Frontera (mcp.yml)**: **sí**.

### E11-H02 — `graph_query`: `path_between` · `cycles` · `components`
- **Objetivo**: las operaciones estructurales nuevas del grafo, como funciones puras del core.
- **Referencias**: `ARCHITECTURE.md §19.6` · `REFACTOR §9.5` · `core::graph`.
- **Alcance**:
  - Funciones puras en `core::graph`: `path_between(a,b)` (camino más corto dirigido),
    `cycles()` (ciclos del grafo de enlaces), `components()` (componentes conexas).
  - Enchufar como operaciones adicionales de `graph_query`.
- **Fuera de alcance**: layout/visualización (UI congelada).
- **Criterios de aceptación**:
  - **Dado** A→B→C, **Cuando** `path_between(A,C)`, **Entonces** `[A,B,C]` → `path_between_directo`.
  - **Dado** A→B→A, **Cuando** `cycles()`, **Entonces** reporta el ciclo `{A,B}` → `detecta_ciclo`.
  - **Dado** dos subgrafos inconexos, **Cuando** `components()`, **Entonces** 2 componentes → `dos_componentes`.
  - **Dado** A y C sin camino, **Cuando** `path_between(A,C)`, **Entonces** vacío (no error) → `sin_camino`.
- **Dependencias**: E11-H01.
- **Pruebas**: `crates/lodestar-core/tests/`: `path_between_directo`, `detecta_ciclo`, `dos_componentes`,
  `sin_camino`.
- **Frontera (mcp.yml)**: **sí** (amplía el enum `operation`).

### E11-H03 — Relaciones tipadas: validación contra el schema (`REL-TARGET`/`REL-CARD`/`REL-TYPE`)
- **Objetivo**: validar las relaciones declaradas en el frontmatter contra su `RelationDef`.
- **Referencias**: `ARCHITECTURE.md §19.2, §19.3` · `REFACTOR §4.2, §10, §17` · `core::schema` (E10-H05), E10-H06.
- **Alcance**:
  - Función pura `validate_relations(bundle, schema) -> Vec<Check>`: por cada relación tipada del
    frontmatter, comprobar (1) el target existe (`REL-TARGET`), (2) la cardinalidad respeta `RelationDef`
    (`REL-CARD`), (3) el `type` del target está en `target_types` (`REL-TYPE`). Todos con `range` al campo.
  - Integrar aditivamente en `analyze`/`knowledge_check` (un bundle sin relaciones tipadas no cambia).
- **Fuera de alcance**: paths de código (E11-H04); planificar cambios de relación (E12).
- **Criterios de aceptación**:
  - **Dado** una relación `appears_in` a un target inexistente, **Cuando** se valida, **Entonces**
    `REL-TARGET` (`Err`) con `range` al campo → `relacion_target_roto` (benchmark §17: "Introducir una
    relación inválida → error antes de escribir").
  - **Dado** una relación a un concepto de `type` no permitido por `RelationDef`, **Cuando** se valida,
    **Entonces** `REL-TYPE` → `relacion_tipo_invalido`.
  - **Dado** una relación de cardinalidad `one` con dos targets, **Cuando** se valida, **Entonces**
    `REL-CARD` → `relacion_cardinalidad`.
- **Dependencias**: E10-H05, E10-H06.
- **Pruebas**: `crates/lodestar-core/tests/`: `relacion_target_roto`, `relacion_tipo_invalido`,
  `relacion_cardinalidad`; fixture con schema de relaciones.
- **Frontera (mcp.yml)**: no (alimenta `knowledge_check`).

### E11-H04 — Validación de paths externos (`referenceRoots`)
- **Objetivo**: comprobar que los paths a código (`implemented_by`/`verified_by`) existen, sin editarlos.
- **Referencias**: `ARCHITECTURE.md §19.4, §19.7` · `REFACTOR §4.2, §17` · E9-H05 (`referenceRoots`).
- **Alcance**:
  - En `workspace` (I/O): resolver los paths de frontmatter que apuntan a `referenceRoots` y comprobar
    existencia; el core recibe el resultado (existe/no) y emite el diagnóstico (nuevo o reutiliza `LINK-REL`).
  - `knowledge_get` expone `externalReferences:[{path,exists}]` (ya previsto en E10-H10).
  - **Nunca** se escribe en `referenceRoots` (inmutables por el MCP).
- **Fuera de alcance**: validar el *contenido* del código (fuera de scope).
- **Criterios de aceptación**:
  - **Dado** un concepto con `implemented_by: [src/x.rs]` inexistente, **Cuando** se valida, **Entonces**
    un diagnóstico de referencia externa rota → `ref_externa_rota` (benchmark §17: "Referenciar un archivo
    de código inexistente → diagnóstico").
  - **Dado** un `implemented_by` a un fichero real bajo `referenceRoots`, **Cuando** se valida, **Entonces**
    `exists:true` y sin diagnóstico → `ref_externa_ok`.
  - **Dado** un intento de escritura sobre `referenceRoots`, **Cuando** se procesa, **Entonces**
    `PERMISSION_DENIED` → `reference_roots_inmutable`.
- **Dependencias**: E9-H05, E10-H10.
- **Pruebas**: `crates/lodestar-workspace/tests/`: `ref_externa_rota`, `ref_externa_ok`,
  `reference_roots_inmutable`.
- **Frontera (mcp.yml)**: no.

### E11-H05 — `impact_analyze` (reusa blast-radius + neighborhood)
- **Objetivo**: analizar un cambio hipotético sin crear un change set, con riesgo y bloqueos.
- **Referencias**: `ARCHITECTURE.md §19.6` · `REFACTOR §9.6, §17` · `Store::blast_radius` (`store/lib.rs:296`),
  `Bundle::neighborhood`.
- **Alcance**:
  - Servicio `App::impact_analyze(ref, proposedOperation{kind}, depth)` → `{ summary{directlyAffected,
    transitivelyAffected,blockingReferences,risk}, affectedConcepts, blockingReferences[{path,reason}],
    recommendations }`.
  - `kind` ∈ `move`/`delete`/`deprecate`/`transition_status`/`change_relation`/`replace_concept`.
  - `directlyAffected` = backlinks directos; `transitivelyAffected` = blast-radius (CTE del store,
    verificado idéntico al `neighborhood(In)` del core); `blockingReferences` = relaciones obligatorias
    que quedarían rotas.
- **Fuera de alcance**: aplicar el cambio (E12/E13).
- **Criterios de aceptación**:
  - **Dado** un concepto con 30 backlinks, **Cuando** `impact_analyze(kind:move)`, **Entonces**
    `directlyAffected == 30` → `impacto_move_30` (benchmark §17: "Mover un concepto con 30 backlinks").
  - **Dado** un concepto con 3 relaciones obligatorias entrantes, **Cuando** `impact_analyze(kind:delete)`,
    **Entonces** `blockingReferences.len() == 3` y `risk` alto → `impacto_delete_bloqueos` (benchmark §17:
    "Borrar un concepto referenciado → rechazo con blockers").
  - **Dado** el blast-radius del store, **Cuando** se compara con `neighborhood(In)` del core, **Entonces**
    idénticos → `impacto_paridad_core`.
- **Dependencias**: E11-H01, E11-H03.
- **Pruebas**: `crates/lodestar-mcp/tests/`: `impacto_move_30`, `impacto_delete_bloqueos`,
  `impacto_paridad_core`.
- **Frontera (mcp.yml)**: **sí**.

---

## Orden de construcción (E11)

`E11-H01` (consolidación) primero; `E11-H02` la amplía. En paralelo, `E11-H03` (relaciones tipadas) y
`E11-H04` (paths externos) dependen del schema de E10. `E11-H05` (impact_analyze) cierra: necesita el
grafo consolidado (H01) y las relaciones tipadas (H03) para los `blockingReferences`. Ninguna
**[BLOQUEADA]**.
