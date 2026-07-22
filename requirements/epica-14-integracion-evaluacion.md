# E14 — Integración con software y evaluación

> **Fase**: `§19.8` fases 5+6 (`REFACTOR §16`). **Objetivo de la épica**: cerrar el giro comprobando que
> la base de conocimiento **convive con el código** sin que Lodestar gestione git ni edite el proyecto
> (validación de paths de código en CI, `knowledge_check` como puerta de CI, config por proyecto,
> instrucciones para agentes), y **evaluar** el motor con el **benchmark funcional** (`REFACTOR §17`) y sus
> métricas.
> Criterios de salida (`REFACTOR §16`): *la base de conocimiento puede convivir con el código sin que
> Lodestar gestione Git ni edite el proyecto* (fase 5) y *el motor está medido y optimizado* (fase 6).
> Referencias maestras: `ARCHITECTURE.md §19.6`, `§19.7` · `REFACTOR §5.3, §7, §12, §16 fases 5–6, §17`.

**Principio rector de la épica**: *medir antes de optimizar, y validar sin poseer el proyecto*. La CLI es
la puerta (no el MCP) para importación/exportación/mantenimiento (`REFACTOR §12`); el benchmark §17 es el
juez de que las 10 tools cubren los escenarios de producto.

---

### E14-H01 — `knowledge_check` como puerta de CI (CLI, sobre el working tree)
- **Objetivo**: `lodestar check` corre la conformidad completa (incluida la schema-driven) como gate de CI.
- **Referencias**: `ARCHITECTURE.md §19.6` · decisión **D-check** · `REFACTOR §12, §16 fase 5` · E10-H12, E11-H03/H04.
- **Alcance**:
  - `lodestar check` (working tree, sin flags git) juzga con el mismo motor que `knowledge_check` scope
    `workspace` (OKF + SCHEMA-* + REL-* + refs externas). Salida humana / `--json` / SARIF.
  - Exit codes: `0` conforme · `1` hard-fail (gate) · `2` uso · `3` runtime/IO.
- **Fuera de alcance**: `--staged/--rev/--range` (diferidos con vcs, D-check).
- **Criterios de aceptación**:
  - **Dado** un bundle con un `SCHEMA-REQFIELD`, **Cuando** se corre `lodestar check`, **Entonces** exit `1`
    → `check_falla_schema`.
  - **Dado** un bundle conforme con schema, **Cuando** se corre `lodestar check --json`, **Entonces** exit
    `0` y JSON con `conformant:true` → `check_conforme_json`.
  - **Dado** un `.md` editado a mano e inválido, **Cuando** corre CI, **Entonces** la puerta lo caza →
    `check_caza_edicion_directa` (benchmark §17: "Editar directamente un Markdown inválido → detectado").
- **Dependencias**: E10-H12, E11-H03, E11-H04.
- **Pruebas**: `crates/lodestar-cli/tests/`: `check_falla_schema`, `check_conforme_json`, `check_caza_edicion_directa`.
- **Frontera (mcp.yml)**: no.

### E14-H02 — Convivencia con proyectos de software (config por proyecto + detección de escrituras externas)
- **Objetivo**: que la base viva dentro de un repo de código sin que Lodestar toque el código ni git.
- **Referencias**: `ARCHITECTURE.md §19.4, §19.7` · `REFACTOR §4.1, §5.3, §16 fase 5` · E9-H05, E11-H04.
- **Alcance**:
  - Config por proyecto: `writableRoots` (p. ej. `knowledge`) separada de `referenceRoots` (`src`/`tests`),
    con `ignored` para `node_modules`/`target`/`.git`.
  - Detección de **escrituras externas** (`REFACTOR §5.3`): al reabrir/tras evento, recalcular/invalidar
    revisiones y reindexar; detectar conflictos antes de aplicar un plan (no asumir acceso exclusivo).
- **Fuera de alcance**: gestionar git del proyecto (fuera de superficie).
- **Criterios de aceptación**:
  - **Dado** un repo con `knowledge/` (writable) y `src/` (reference), **Cuando** se abre, **Entonces**
    Lodestar solo escribe bajo `knowledge/` → `solo_escribe_writable`.
  - **Dado** que un agente externo editó un `.md` writable entre el plan y el apply, **Cuando** se aplica,
    **Entonces** el conflicto se detecta (`REVISION_CONFLICT`/`WRITE_CONFLICT`) → `detecta_escritura_externa`.
- **Dependencias**: E9-H05, E11-H04, E13-H08.
- **Pruebas**: `crates/lodestar-workspace/tests/`: `solo_escribe_writable`, `detecta_escritura_externa`.
- **Frontera (mcp.yml)**: no.

### E14-H03 — Instrucciones del servidor + perfiles para agentes
- **Objetivo**: orientar al agente con el flujo recomendado y exponer las capacidades por perfil.
- **Referencias**: `ARCHITECTURE.md §19.6` · `REFACTOR §7, §12`.
- **Alcance**:
  - Server instructions del MCP con el flujo `workspace_status → knowledge_search → knowledge_get →
    schema_inspect → graph_query/impact_analyze → change_plan → change_apply → knowledge_check → change_revert`.
  - Perfiles `readonly`/`standard` seleccionables al arrancar (`--profile`); `workspace_status.capabilities`
    coherente con el perfil.
- **Fuera de alcance**: prompts MCP / sampling (no-goal, `REFACTOR §16`).
- **Criterios de aceptación**:
  - **Dado** el servidor con `--profile readonly`, **Cuando** un cliente pide `tools/list`, **Entonces** no
    aparecen las 3 tools de cambio → `perfil_readonly_sin_cambio`.
  - **Dado** el arranque, **Cuando** el cliente lee las instrucciones, **Entonces** describen el flujo de 10
    pasos → `instrucciones_flujo`.
- **Dependencias**: E10-H08, E13-H08.
- **Pruebas**: `crates/lodestar-mcp/tests/`: `perfil_readonly_sin_cambio`, `instrucciones_flujo`.
- **Frontera (mcp.yml)**: **sí** (perfiles/instrucciones).

### E14-H04 — Benchmark funcional (`REFACTOR §17`) como suite e2e
- **Objetivo**: los 15 escenarios de producto como test e2e que cruzan MCP y CLI.
- **Referencias**: `ARCHITECTURE.md §19.8` · `REFACTOR §17`.
- **Alcance**:
  - Suite e2e que ejecuta los 15 escenarios de la tabla §17 (encontrar por significado, crear válido/inválido,
    mover con 30 backlinks, borrar referenciado, `REVISION_CONFLICT`, 5 conceptos en un change set, relación
    inválida, safe fixes, diff de refactor, revert, crash durante publicación, escribir fuera de writable,
    ref de código inexistente, editar Markdown inválido).
  - Muchos escenarios ya tienen su test unitario en E10–E13; esta historia los **compone** como viaje e2e.
- **Fuera de alcance**: métricas de rendimiento (E14-H05).
- **Criterios de aceptación**:
  - **Dado** el bundle de benchmark, **Cuando** se corre la suite, **Entonces** los 15 escenarios dan el
    resultado esperado de §17 → `benchmark_15_escenarios` (un test por fila o un test tabular).
  - **Dado** el escenario de crash, **Cuando** se ejecuta con `FailPoint`, **Entonces** la recuperación es
    determinista → reutiliza `recovery_sin_parciales` (E13-H06).
- **Dependencias**: E13-H09 (todas las tools listas).
- **Pruebas**: `crates/lodestar-mcp/tests/benchmark.rs` (o `lodestar-app`): `benchmark_15_escenarios`.
- **Frontera (mcp.yml)**: no.

### E14-H05 — Métricas de evaluación y presupuesto de escala
- **Objetivo**: medir tokens/tool-calls/concurrencia/recuperación/latencia en workspaces grandes.
- **Referencias**: `ARCHITECTURE.md §19.8`, `§11` (presupuesto) · `REFACTOR §16 fase 6, §17` (métricas).
- **Alcance**:
  - Arnés de medición sobre una fixture sintética grande (p. ej. 10k conceptos): latencia de
    `knowledge_search`/`graph_query`/`impact_analyze`/`change_plan`; tamaño de payload (proxy de tokens);
    concurrencia (dos publicadores → lock); recuperación (tiempo de crash-recovery).
  - Umbrales orientativos como gate opcional (no bloquea v2): reusar el presupuesto de `§11` donde aplique.
- **Fuera de alcance**: optimización agresiva (aditiva, guiada por las mediciones).
- **Criterios de aceptación**:
  - **Dado** una fixture de 10k conceptos, **Cuando** se mide `knowledge_search`, **Entonces** la latencia se
    registra y no devuelve cuerpos completos (payload acotado) → `bench_search_payload_acotado`.
  - **Dado** dos `change_apply` concurrentes, **Cuando** se ejecutan, **Entonces** uno gana el lock y el otro
    recibe `WRITE_CONFLICT` (sin corrupción) → `bench_concurrencia_segura`.
- **Dependencias**: E14-H04.
- **Pruebas**: `crates/lodestar-mcp/tests/` o bench: `bench_search_payload_acotado`, `bench_concurrencia_segura`.
- **Frontera (mcp.yml)**: no.

### E14-H06 — Retirada de la superficie heredada (10 tools heredadas → 10 objetivo)
- **Objetivo**: converger la superficie MCP a las **10 tools objetivo** del giro, retirando las 10 tools
  heredadas cuya migración (`§15`) quedó diferida hasta tener todos los reemplazos. Es el "único rewrite"
  que anticipa la nota de `contracts/mcp.yml §15` (no pasos parciales).
- **Referencias**: `ARCHITECTURE.md §19.6` · `REFACTOR §8, §15, §16` · `contracts/mcp.yml §15` (tabla de
  reemplazos) · `DECISIONES.md §0`. Dependencias: E10–E13 (todos los reemplazos ya existen y verificados).
- **Alcance**:
  - Retirar de `tools/list` y del despacho de `lodestar-mcp` las 10 heredadas: `query`,
    `conformance_check`, `find_backlinks`, `find_orphans`, `find_dangling`, `neighborhood`,
    `create_concept`, `update_frontmatter`, `generate_index`, `generate_tag_indexes`.
  - Superficie resultante: EXACTAMENTE las 10 objetivo (`workspace_status`, `knowledge_search`,
    `knowledge_get`, `schema_inspect`, `graph_query`, `impact_analyze`, `knowledge_check`, `change_plan`,
    `change_apply`, `change_revert`).
  - `contracts/mcp.yml`: reescribir para que la sección `tools:` liste solo las 10; las heredadas pasan a
    `§15` como **retiradas** (con su reemplazo semántico). Actualizar el recuento narrativo a 10.
  - Migrar/retirar los tests que ejercitaban las tools heredadas: su cobertura equivalente ya vive en los
    tests de las tools objetivo (`knowledge_search`/`graph_query`/`knowledge_check`/`change_plan`+`apply`).
    La capacidad NO se pierde (retira exposición, no capacidad — `lodestar-core`/`workspace` conservan la
    mecánica; la CLI mantiene `index`/`tags`).
- **Fuera de alcance**: eliminar código de dominio de `core`/`workspace` (la mecánica se conserva); tocar
  la CLI (mantiene `index`/`tags`/`check`); `frontend/`/`src-tauri/` (congelados).
- **Criterios de aceptación**:
  - **Dado** el servidor MCP, **Cuando** un cliente pide `tools/list` (perfil standard), **Entonces**
    devuelve EXACTAMENTE las 10 tools objetivo y NINGUNA heredada → `tools_list_solo_objetivo`.
  - **Dado** el servidor, **Cuando** un cliente invoca una tool heredada (p. ej. `query`/`conformance_check`/
    `find_backlinks`/`create_concept`/`generate_index`), **Entonces** se rechaza como parámetro inválido
    (`-32602`: `tools/call` es método válido, el nombre de tool desconocido es un parámetro) sin ejecutar →
    `tool_heredada_retirada`. (Convención JSON-RPC coherente con la retirada de las tools git en `E9-H01`:
    `call_commit_desconocida` → `-32602`.)
- **Dependencias**: E10-H09/H12 (search/check), E11-H01 (graph_query), E12-H08/E13-H08/H09 (change_*),
  E13-H11 (auto-regen sustituye generate_*). Ninguna **[BLOQUEADA]** (todos los reemplazos existen).
- **Pruebas**: `crates/lodestar-mcp/tests/`: `tools_list_solo_objetivo`, `tool_heredada_retirada`.
- **Frontera (mcp.yml)**: **sí** (retirada de 10 tools; reescritura de la superficie a 10).

---

## Orden de construcción (E14)

`E14-H01` (gate de CI) y `E14-H02` (convivencia) se pueden construir en cuanto E10–E13 den sus piezas.
`E14-H03` (perfiles/instrucciones) necesita las tools completas. `E14-H04` (benchmark e2e) compone todo lo
anterior y por eso va casi al final; `E14-H05` (métricas) cierra sobre el benchmark. `E14-H06` (retirada de
la superficie heredada) va **la última**: converge a las 10 tools objetivo una vez que el benchmark ha
demostrado que las nuevas cubren los 15 escenarios. Ninguna historia **[BLOQUEADA]**.
