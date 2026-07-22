# E12 — Planificación de cambios

> **Fase**: `§19.8` fase 3 (`REFACTOR §16`). **Objetivo de la épica**: que un agente pueda **proponer
> refactors complejos sin modificar archivos**. Entrega los tipos del plan (`ChangeSet`/
> `NormalizedOperation`/`RiskAssessment`/`SemanticDiff`/`ValidationReport`), la **normalización de las 11
> operaciones**, y la tool central **`change_plan`** que normaliza, **simula en staging virtual** y valida
> una propuesta con **control optimista de concurrencia**, persistiendo el plan en `.lodestar/runtime/plans/`.
> Criterio de salida (`REFACTOR §16`): *un agente puede proponer refactors complejos sin modificar archivos*.
> Referencias maestras: `ARCHITECTURE.md §19.3`, `§19.5`, `§19.6` · `REFACTOR §6.4, §11.1, §17` ·
> `Mutation` (`core/types.rs:430`), `OkfDiff`/`diff_snap` (`core::diff`), `FrontmatterPatch`.

**Principio rector de la épica**: *`change_plan` NO escribe*. Toda la simulación ocurre sobre un
`Bundle` en memoria (el resultado hipotético), no en disco. El plan es un artefacto **runtime** (no
canónico, invariante #1). La escritura real es E13. Un plan reproducible: mismo input + misma
`baseWorkspaceRevision` ⇒ mismo `planHash`.

---

### E12-H01 — Tipos del plan en `core::types` (`ChangeSet`, `NormalizedOperation`, ids/hashes)
- **Objetivo**: congelar los tipos del plan como parte del contrato único.
- **Referencias**: `ARCHITECTURE.md §19.3` · `REFACTOR §6.4`.
- **Alcance**:
  - `ChangeSetId`/`PlanHash`/`ReceiptId` (newtypes tipo `Sha`); `ChangeSet { id, base_revision:
    WorkspaceRevision, operations: Vec<NormalizedOperation>, plan_hash, risk, semantic_diff, validation,
    expires_at }`.
  - `enum NormalizedOperation` con las 11 variantes (contenido/estructura/semántica), cada una **resuelta**
    a las escrituras/borrados concretos que producirá (un `Mutation` derivable).
- **Fuera de alcance**: normalizar de verdad cada op (E12-H05…H07); riesgo/diff (E12-H02/H03).
- **Criterios de aceptación**:
  - **Dado** un `ChangeSet` serializado, **Cuando** se inspecciona, **Entonces** lleva `baseWorkspaceRevision`,
    `planHash`, `expiresAt` (wire camelCase) → `changeset_shape`.
  - Estructural: no existe otra definición de estos tipos fuera de `core::types` (grep en CI).
- **Dependencias**: E10-H03.
- **Pruebas**: `crates/lodestar-core/tests/`: `changeset_shape`; round-trip serde.
- **Frontera (mcp.yml)**: no.

### E12-H02 — `RiskAssessment` (lógica pura de riesgo)
- **Objetivo**: derivar un nivel de riesgo con razones a partir del alcance del cambio.
- **Referencias**: `ARCHITECTURE.md §19.3` · `REFACTOR §6.4, §11.1` · blast-radius/backlinks (E11).
- **Alcance**:
  - Función pura `assess_risk(ops, bundle_before, bundle_after) -> RiskAssessment { level: Low|Medium|High,
    reasons: Vec<String> }`. Heurística documentada: deprecar/borrar conceptos con muchos backlinks = alto;
    cambios aislados = bajo.
- **Fuera de alcance**: i18n de las razones (claves en la fachada).
- **Criterios de aceptación**:
  - **Dado** un `deprecate` sobre un concepto con 7 backlinks, **Cuando** se evalúa, **Entonces**
    `level >= Medium` con una razón que lo menciona → `riesgo_deprecate_backlinks`.
  - **Dado** un `patch_frontmatter` sin backlinks afectados, **Cuando** se evalúa, **Entonces** `level: Low`
    → `riesgo_bajo_aislado`.
- **Dependencias**: E12-H01, E11-H05.
- **Pruebas**: `crates/lodestar-core/tests/`: `riesgo_deprecate_backlinks`, `riesgo_bajo_aislado`.
- **Frontera (mcp.yml)**: no.

### E12-H03 — `SemanticDiff` (reusa `OkfDiff` + diagnósticos introducidos/resueltos)
- **Objetivo**: el diff semántico entre el estado actual y el hipotético del plan.
- **Referencias**: `ARCHITECTURE.md §19.3` · `REFACTOR §6.4, §11.1` · `core::diff` (`diffSnap`), E10-H07.
- **Alcance**:
  - `SemanticDiff { created, modified, deleted, moved, frontmatterChanges, bodyChanges, relationChanges,
    diagnosticsIntroduced, diagnosticsResolved }`. `created/modified/…/frontmatterChanges/bodyChanges`
    reutilizan `OkfDiff`; `diagnosticsIntroduced/Resolved` = diff de `Vec<Check>` (después − antes).
- **Fuera de alcance**: render (UI congelada).
- **Criterios de aceptación**:
  - **Dado** un plan que crea A y modifica B, **Cuando** se computa el diff, **Entonces**
    `created==[A]` y `modified==[B]` → `diff_created_modified`.
  - **Dado** un plan que corrige un `SCHEMA-REQFIELD`, **Cuando** se computa, **Entonces** ese diagnóstico
    aparece en `diagnosticsResolved` → `diff_resuelve_diagnostico` (benchmark §17: "Revisar un refactor →
    diff semántico en change_plan").
  - **Dado** un plan que rompe una relación, **Cuando** se computa, **Entonces** aparece en
    `diagnosticsIntroduced` → `diff_introduce_diagnostico`.
- **Dependencias**: E12-H01, E10-H07.
- **Pruebas**: `crates/lodestar-core/tests/`: `diff_created_modified`, `diff_resuelve_diagnostico`,
  `diff_introduce_diagnostico`.
- **Frontera (mcp.yml)**: no.

### E12-H04 — `ValidationReport` (conformidad del resultado hipotético)
- **Objetivo**: el veredicto de conformidad del bundle resultante del plan, con política.
- **Referencias**: `ARCHITECTURE.md §19.3` · `REFACTOR §10, §11.1` · `Analysis`, `Config.gate`.
- **Alcance**:
  - `ValidationReport { conformant, summary{errors,warnings,info}, diagnostics: Vec<Check> }` sobre el
    `analyze()` del bundle hipotético.
  - `policy { requireConformantResult, allowWarnings }` decide `canApply` (`REFACTOR §11.1`).
- **Fuera de alcance**: aplicar (E13).
- **Criterios de aceptación**:
  - **Dado** un plan cuyo resultado introduce un `Err` y `policy.requireConformantResult:true`, **Cuando**
    se valida, **Entonces** `conformant:false` y el plan no es aplicable → `plan_no_conforme_rechaza`
    (benchmark §17: "Crear un concepto sin campo obligatorio → plan rechazado").
  - **Dado** un plan con solo warnings y `allowWarnings:true`, **Cuando** se valida, **Entonces**
    `canApply:true` → `plan_warnings_permitido`.
- **Dependencias**: E12-H01, E10-H07.
- **Pruebas**: `crates/lodestar-core/tests/`: `plan_no_conforme_rechaza`, `plan_warnings_permitido`.
- **Frontera (mcp.yml)**: no.

### E12-H05 — Normalización de operaciones de contenido
- **Objetivo**: normalizar `create`/`patch_frontmatter`/`replace_body`/`edit_section`/`replace_text` a escrituras concretas.
- **Referencias**: `ARCHITECTURE.md §19.3` · `REFACTOR §11.1` · `Bundle::create_concept`/`merge_frontmatter`
  (`bundle.rs:204-205`), `FrontmatterPatch`, plantillas (E10-H05).
- **Alcance**:
  - Normalizador puro que, dado un `Bundle` y una op, produce la `NormalizedOperation` resuelta (path,
    contenido final). `create` usa la `bodyTemplate` del `DocType` si no se da body; `patch_frontmatter`
    usa `FrontmatterPatch` (null-borra); `edit_section` por `headingPath`+`mode`; `replace_text` con
    `expectedOccurrences` (falla si no casa el número).
- **Fuera de alcance**: estructura (E12-H06); semántica (E12-H07).
- **Criterios de aceptación**:
  - **Dado** `create` sin body para un `DocType` con `bodyTemplate`, **Cuando** se normaliza, **Entonces**
    el cuerpo sale de la plantilla → `create_usa_plantilla`.
  - **Dado** `replace_text` con `expectedOccurrences:1` y 2 coincidencias, **Cuando** se normaliza,
    **Entonces** error (no aplica) → `replace_text_ocurrencias`.
  - **Dado** `edit_section(["Security","Token rotation"], mode:replace)`, **Cuando** se normaliza,
    **Entonces** solo esa subsección cambia → `edit_section_acotado`.
- **Dependencias**: E12-H01, E10-H05.
- **Pruebas**: `crates/lodestar-core/tests/`: `create_usa_plantilla`, `replace_text_ocurrencias`,
  `edit_section_acotado`.
- **Frontera (mcp.yml)**: no.

### E12-H06 — Normalización de operaciones de estructura (`move`, `delete`)
- **Objetivo**: normalizar movimientos y borrados con reescritura de enlaces y políticas de entrantes.
- **Referencias**: `ARCHITECTURE.md §19.3` · `REFACTOR §11.1` (move/delete/políticas), `§17`.
- **Alcance**:
  - `move { ref, destination, rewriteInboundLinks }`: normaliza el rename + (si procede) la reescritura de
    los enlaces entrantes dentro del **mismo** change set.
  - `delete { ref, inboundLinksPolicy }` con políticas `reject`(default)/`retarget`/`remove_links`/`create_stub`.
- **Fuera de alcance**: aplicar el rename en disco (E13).
- **Criterios de aceptación**:
  - **Dado** `move` con `rewriteInboundLinks:true` y 30 backlinks, **Cuando** se normaliza, **Entonces**
    el change set incluye la reescritura de los 30 → `move_reescribe_entrantes` (benchmark §17: "Mover un
    concepto con 30 backlinks → enlaces actualizados dentro del mismo plan").
  - **Dado** `delete` con `inboundLinksPolicy` por defecto sobre un concepto referenciado, **Cuando** se
    normaliza, **Entonces** se rechaza con `INBOUND_LINKS_EXIST` (default `reject`) → `delete_referenciado_rechaza`.
  - **Dado** `delete` con `remove_links` sobre un concepto referenciado, **Cuando** se normaliza,
    **Entonces** el change set incluye quitar esos enlaces → `delete_remove_links`.
- **Dependencias**: E12-H01, E11-H01.
- **Pruebas**: `crates/lodestar-core/tests/`: `move_reescribe_entrantes`, `delete_referenciado_rechaza`,
  `delete_remove_links`.
- **Frontera (mcp.yml)**: no.

### E12-H07 — Normalización de operaciones semánticas (`add_relation`/`remove_relation`/`transition_status`/`apply_fix`)
- **Objetivo**: normalizar las operaciones sobre relaciones, ciclo de vida y fixes sugeridos.
- **Referencias**: `ARCHITECTURE.md §19.3` · `REFACTOR §11.1` · relaciones tipadas (E11-H03), fixes (E10-H12).
- **Alcance**:
  - `add_relation`/`remove_relation { source, relation, target }`: normaliza el patch de frontmatter,
    validando contra `RelationDef` (rechaza `RELATION_CONSTRAINT_VIOLATION` si viola el schema).
  - `transition_status { ref, to }`: valida contra `allowedStatuses`/lifecycle.
  - `apply_fix { fixId }`: materializa un fix `safe` de un diagnóstico previo.
- **Fuera de alcance**: aplicar (E13).
- **Criterios de aceptación**:
  - **Dado** `add_relation` que viola `RelationDef` (tipo de target no permitido), **Cuando** se normaliza,
    **Entonces** `RELATION_CONSTRAINT_VIOLATION` → `add_relation_invalida` (benchmark §17: "Introducir una
    relación inválida → error antes de escribir").
  - **Dado** `transition_status` a un estado no permitido, **Cuando** se normaliza, **Entonces** rechazo →
    `transicion_invalida`.
  - **Dado** `apply_fix` con el `fixId` de un fix `safe`, **Cuando** se normaliza, **Entonces** produce la
    escritura correctora → `apply_fix_safe` (benchmark §17: "Corregir safe fixes → operaciones apply_fix").
- **Dependencias**: E12-H01, E11-H03, E10-H12.
- **Pruebas**: `crates/lodestar-core/tests/`: `add_relation_invalida`, `transicion_invalida`, `apply_fix_safe`.
- **Frontera (mcp.yml)**: no.

### E12-H08 — `change_plan` (orquestación: normaliza + simula + valida, sin escribir)
- **Objetivo**: la tool central que ensambla el `ChangeSet` completo sin tocar disco.
- **Referencias**: `ARCHITECTURE.md §19.5, §19.6` · `REFACTOR §11.1, §17`.
- **Alcance**:
  - Servicio `App::change_plan(expectedWorkspaceRevision, operations, policy)` → `{ changeSetId,
    baseWorkspaceRevision, planHash, canApply, expiresAt, normalizedOperations, risk, semanticDiff, impact,
    diagnosticsBefore, diagnosticsAfter }`.
  - Pasos: verificar `expectedWorkspaceRevision` (o tomar la actual) → normalizar todas las ops sobre un
    `Bundle` en memoria → construir el bundle hipotético → `SemanticDiff` + `RiskAssessment` +
    `ValidationReport` → `planHash` determinista.
  - **Control optimista**: cada op con `expectedRevision` verifica que el concepto no cambió; discrepancia
    → `REVISION_CONFLICT`.
  - Tool MCP `change_plan` (perfil `standard`) con `inputSchema`/`outputSchema`.
- **Fuera de alcance**: persistir el plan (E12-H09); aplicar (E13).
- **Criterios de aceptación**:
  - **Dado** una propuesta de 5 conceptos relacionados, **Cuando** se planifica, **Entonces** un **único**
    `ChangeSet` con `normalizedOperations` de los 5 → `plan_un_solo_changeset` (benchmark §17: "Cambiar
    cinco conceptos relacionados → un único change set").
  - **Dado** un `expectedRevision` de un concepto cambiado externamente, **Cuando** se planifica, **Entonces**
    `REVISION_CONFLICT` → `plan_revision_conflict` (benchmark §17: "Modificar un concepto cambiado
    externamente → REVISION_CONFLICT").
  - **Dado** el mismo input y `baseWorkspaceRevision`, **Cuando** se planifica dos veces, **Entonces** el
    `planHash` coincide → `plan_hash_determinista`.
  - **Dado** un `change_plan`, **Cuando** termina, **Entonces** el disco **no** cambió → `plan_no_escribe`.
- **Dependencias**: E12-H02, E12-H03, E12-H04, E12-H05, E12-H06, E12-H07, E11-H05.
- **Pruebas**: `crates/lodestar-mcp/tests/`: `plan_un_solo_changeset`, `plan_revision_conflict`,
  `plan_hash_determinista`, `plan_no_escribe`.
- **Frontera (mcp.yml)**: **sí**.

### E12-H09 — Persistencia del plan en `.lodestar/runtime/plans/`
- **Objetivo**: guardar cada plan (operaciones normalizadas, revisión base, hash, caducidad, diff, impacto, validación).
- **Referencias**: `ARCHITECTURE.md §19.4, §19.5` · `REFACTOR §11.1` (persistencia del plan), `§4.1`.
- **Alcance**:
  - Escribir `.lodestar/runtime/plans/<changeSetId>.json` con el `ChangeSet` completo. Es **runtime**
    (gitignored, no canónico, fuera de `WorkspaceRevision`).
  - Caducidad: `expiresAt`; cargar un plan caducado → `PLAN_EXPIRED`.
- **Fuera de alcance**: aplicar el plan (E13-H08).
- **Criterios de aceptación**:
  - **Dado** un `change_plan` exitoso, **Cuando** termina, **Entonces** existe
    `.lodestar/runtime/plans/<id>.json` con el `planHash` → `plan_persistido`.
  - **Dado** un plan con `expiresAt` en el pasado, **Cuando** se carga, **Entonces** `PLAN_EXPIRED` →
    `plan_caducado`.
  - **Dado** el plan persistido, **Cuando** se calcula `WorkspaceRevision`, **Entonces** el plan no la
    afecta (es runtime) → `plan_fuera_de_revision`.
- **Dependencias**: E12-H08, E9-H06.
- **Pruebas**: `crates/lodestar-app/tests/` o `workspace`: `plan_persistido`, `plan_caducado`,
  `plan_fuera_de_revision`.
- **Frontera (mcp.yml)**: no.

---

## Orden de construcción (E12)

Tipos primero (`E12-H01`), luego las tres piezas de análisis del plan (`E12-H02` riesgo, `E12-H03` diff,
`E12-H04` validación) y las tres de normalización (`E12-H05` contenido, `E12-H06` estructura, `E12-H07`
semántica) — todas paralelizables tras H01. `E12-H08` (`change_plan`) las integra y depende de
`E11-H05`. `E12-H09` (persistencia) cierra sobre H08 + `E9-H06`. Ninguna **[BLOQUEADA]**.
