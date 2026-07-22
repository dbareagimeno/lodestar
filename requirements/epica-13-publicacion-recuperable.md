# E13 — Publicación recuperable

> **Fase**: `§19.8` fase 4 (`REFACTOR §16`). **Objetivo de la épica**: que los cambios complejos se
> **publiquen de forma recuperable** y que **ningún fallo deje corrupción silenciosa**. Implementa la
> **mecánica transaccional** en `lodestar-workspace` (por el ÚNICO escritor): staging, write-ahead
> journal, lock de workspace, copias de recuperación, aplicación atómica por lote, **crash-recovery
> determinista**, receipts y auditoría; y las tools **`change_apply`** / **`change_revert`**.
> Criterio de salida (`REFACTOR §16`): *los cambios complejos se publican de forma recuperable y los
> fallos no dejan corrupción silenciosa*.
> Referencias maestras: `ARCHITECTURE.md §19.5`, `§19.2`, `§19.7` · `REFACTOR §5, §11.2, §11.3, §14, §17` ·
> `write_atomic` (`workspace/io.rs:63`), `Mutation`, invariante #5 (único escritor), #1 (runtime ≠ canónico).

**Principio rector de la épica**: *nunca un estado parcial silencioso*. Cada historia deja una **invariante
de recuperación** testeada: si el proceso muere en cualquier punto, al reabrir el workspace se **completa**
o **restaura** de forma determinista, sin `.md` a medias. Staging/journal/receipts/audit viven en
`.lodestar/runtime/` (desechable, invariante #1). Toda escritura pasa por el único escritor (invariante #5).

**Nota de fixtures**: varias historias usan una **sonda de fallo** inyectable (`FailPoint`) que aborta la
publicación en un paso concreto (tras journal, entre renames, antes del receipt…), para que los tests
maten el proceso a mitad y verifiquen la recuperación. La sonda es solo de test (feature `test-failpoints`).

---

### E13-H01 — Staging: materializar el resultado completo + validar staging
- **Objetivo**: escribir el resultado hipotético del plan en `.lodestar/runtime/staging/` y validarlo antes de publicar.
- **Referencias**: `ARCHITECTURE.md §19.5` · `REFACTOR §5.2` (pasos 2–3), `§11.2` (paso 6–7).
- **Alcance**:
  - `Workspace::materialize_staging(change_set) -> StagingDir`: escribe **todos** los ficheros resultantes
    (writes + borrados marcados) en `.lodestar/runtime/staging/<changeSetId>/`.
  - `validate_staging`: construye un `Bundle` desde el árbol de staging (canónico + staging) y corre
    `analyze` — si el resultado no cumple la política, aborta **sin** tocar el canónico.
- **Fuera de alcance**: journal (H03), publicación real (H05).
- **Criterios de aceptación**:
  - **Dado** un change set de 3 escrituras, **Cuando** se materializa en staging, **Entonces** los 3
    ficheros existen bajo `.lodestar/runtime/staging/<id>/` y el canónico **no** cambió → `staging_no_toca_canonico`.
  - **Dado** un staging que resultaría no conforme (política estricta), **Cuando** se valida, **Entonces**
    aborta con `NONCONFORMANT_RESULT` y limpia el staging → `staging_no_conforme_aborta`.
- **Dependencias**: E12-H09, E9-H06.
- **Pruebas**: `crates/lodestar-workspace/tests/`: `staging_no_toca_canonico`, `staging_no_conforme_aborta`.
- **Frontera (mcp.yml)**: no.

### E13-H02 — Lock de workspace + re-verificación de `WorkspaceRevision` base
- **Objetivo**: control de concurrencia: un solo publicador a la vez y la base no cambió.
- **Referencias**: `ARCHITECTURE.md §19.5` · `REFACTOR §5.2` (bloqueo, control optimista), `§11.2` (pasos 8–9).
- **Alcance**:
  - `WorkspaceLock` (fichero de lock en `.lodestar/runtime/`, con owner/pid/timestamp) adquirido antes de
    publicar; liberado siempre (RAII).
  - Re-verificar que `WorkspaceRevision` sigue siendo la `baseWorkspaceRevision` del plan; si cambió →
    `WRITE_CONFLICT`/`REVISION_CONFLICT` y aborta.
- **Fuera de alcance**: la publicación (H05); recovery (H06).
- **Criterios de aceptación**:
  - **Dado** un lock tomado, **Cuando** otro publicador intenta adquirirlo, **Entonces** falla/espera (no
    dos escritores) → `lock_exclusivo`.
  - **Dado** que el workspace cambió entre plan y apply, **Cuando** se re-verifica la revisión, **Entonces**
    `WRITE_CONFLICT` y no se publica → `revision_base_cambiada`.
  - **Dado** un panic durante la publicación, **Cuando** el guard se dropea, **Entonces** el lock se libera
    (no queda huérfano) → `lock_se_libera_en_panic`.
- **Dependencias**: E10-H03.
- **Pruebas**: `crates/lodestar-workspace/tests/`: `lock_exclusivo`, `revision_base_cambiada`,
  `lock_se_libera_en_panic`.
- **Frontera (mcp.yml)**: no.

### E13-H03 — Write-ahead journal
- **Objetivo**: registrar la intención completa de la transacción antes de tocar el canónico, y cada sustitución.
- **Referencias**: `ARCHITECTURE.md §19.5` · `REFACTOR §5.2` (write-ahead journal), `§11.2` (pasos 8, 10).
- **Alcance**:
  - `Journal` en `.lodestar/runtime/journal/<txnId>.json`: estado (`prepared`→`applying`→`applied`→`done`),
    lista de operaciones (path, temp, backup) y `baseWorkspaceRevision`/`resultWorkspaceRevision` esperados.
  - Escribir el journal `prepared` **antes** de la primera sustitución (fsync); actualizar por cada rename.
- **Fuera de alcance**: leerlo para recuperar (H06).
- **Criterios de aceptación**:
  - **Dado** una transacción a punto de publicar, **Cuando** se prepara, **Entonces** existe el journal en
    estado `prepared` con las N operaciones, fsynced → `journal_prepared_antes_de_publicar`.
  - **Dado** una sustitución completada, **Cuando** se registra, **Entonces** el journal la marca aplicada →
    `journal_registra_cada_rename`.
- **Dependencias**: E13-H01.
- **Pruebas**: `crates/lodestar-workspace/tests/`: `journal_prepared_antes_de_publicar`,
  `journal_registra_cada_rename`.
- **Frontera (mcp.yml)**: no.

### E13-H04 — Copias de recuperación (backup de los originales)
- **Objetivo**: guardar el contenido previo de cada fichero afectado para poder restaurar.
- **Referencias**: `ARCHITECTURE.md §19.5` · `REFACTOR §5.2` (copias de recuperación), `§11.2` (paso 10).
- **Alcance**:
  - Antes de sustituir, copiar el original de cada path afectado a `.lodestar/runtime/recovery/<txnId>/`;
    los paths creados nuevos se marcan como "no existía" (para poder borrarlos al revertir).
  - Referenciadas desde el journal.
- **Fuera de alcance**: usarlas en revert (H09) o recovery (H06).
- **Criterios de aceptación**:
  - **Dado** una transacción que modifica B y crea C, **Cuando** se preparan las copias, **Entonces** existe
    el backup de B y una marca "C no existía" → `backup_originales`.
  - **Dado** un path afectado con contenido X, **Cuando** se hace backup, **Entonces** el backup contiene X
    byte-a-byte → `backup_fiel`.
- **Dependencias**: E13-H03.
- **Pruebas**: `crates/lodestar-workspace/tests/`: `backup_originales`, `backup_fiel`.
- **Frontera (mcp.yml)**: no.

### E13-H05 — Aplicación atómica por lote (único escritor)
- **Objetivo**: publicar el resultado sustituyendo ficheros por renames atómicos, uno a uno, por el único escritor.
- **Referencias**: `ARCHITECTURE.md §19.5` · `REFACTOR §5.2` (reemplazo atómico por archivo), `§11.2` (paso 11) ·
  `write_atomic` (`workspace/io.rs:63`), invariante #5.
- **Alcance**:
  - `Workspace::publish(txn)`: por cada operación, `write_atomic` (temp+fsync+rename) o `delete`, en orden
    determinista, actualizando el journal tras cada una. **No** hay segundo escritor; el watcher absorbe el
    lote auto-originado (gate blake3).
  - Al terminar, marcar journal `applied` y calcular `resultWorkspaceRevision`.
- **Fuera de alcance**: recovery si muere a mitad (H06); receipt (H07).
- **Criterios de aceptación**:
  - **Dado** un change set de 3 escrituras, **Cuando** se publica, **Entonces** los 3 `.md` canónicos
    quedan con el contenido del staging → `publica_lote`.
  - **Dado** la publicación completada, **Cuando** se calcula `WorkspaceRevision`, **Entonces** coincide con
    el `resultWorkspaceRevision` previsto por el plan → `revision_resultante_coincide`.
  - Estructural: la publicación usa `write_atomic` (grep: ningún otro camino de escritura del canónico).
- **Dependencias**: E13-H04.
- **Pruebas**: `crates/lodestar-workspace/tests/`: `publica_lote`, `revision_resultante_coincide`.
- **Frontera (mcp.yml)**: no.

### E13-H06 — Crash-recovery determinista (journal incompleto al abrir)
- **Objetivo**: al abrir, detectar un journal incompleto y **completar** o **restaurar** de forma determinista.
- **Referencias**: `ARCHITECTURE.md §19.5` · `REFACTOR §5.2` (recuperación tras fallo), `§11.2`, `§17`.
- **Alcance**:
  - En `Workspace::open`: si hay un journal no `done`, decidir por su estado: `applied` (todo renombrado) →
    **completar** (limpiar staging/backup, marcar `done`); `applying`/`prepared` → **restaurar** desde las
    copias de recuperación (deshacer renames parciales, borrar los creados).
  - `WORKSPACE_RECOVERY_REQUIRED` bloquea escrituras hasta que la recuperación termina;
    `workspace_status.recovery.pendingTransaction` lo refleja.
  - **Sonda de fallo** (`FailPoint`) para abortar la publicación en cada paso.
- **Fuera de alcance**: revert de usuario (H09).
- **Criterios de aceptación**:
  - **Dado** un fallo inyectado **entre** el rename 1 y el 2 de 3, **Cuando** se reabre, **Entonces** el
    estado queda **como antes** de la transacción (los 3 originales), sin `.md` a medias →
    `recovery_restaura_desde_medio` (benchmark §17: "Cerrar Lodestar durante publicación → recuperación
    determinista").
  - **Dado** un fallo inyectado **tras** el último rename pero **antes** de marcar `done`, **Cuando** se
    reabre, **Entonces** la transacción se **completa** (resultado final, staging limpio) → `recovery_completa`.
  - **Dado** una recuperación en curso, **Cuando** llega una escritura, **Entonces** `WORKSPACE_RECOVERY_REQUIRED`
    → `recovery_bloquea_escritura`.
  - **Dado** cualquier punto de fallo, **Cuando** se reabre, **Entonces** nunca hay un fichero con contenido
    parcial (property test sobre todos los `FailPoint`) → `recovery_sin_parciales`.
- **Dependencias**: E13-H05.
- **Pruebas**: `crates/lodestar-workspace/tests/`: `recovery_restaura_desde_medio`, `recovery_completa`,
  `recovery_bloquea_escritura`, `recovery_sin_parciales` (property con `FailPoint`).
- **Frontera (mcp.yml)**: no.

### E13-H07 — `ChangeReceipt` + retención
- **Objetivo**: registrar cada aplicación completada y retenerla según config para permitir revert.
- **Referencias**: `ARCHITECTURE.md §19.3` · `REFACTOR §6.5, §11.3` · config `transactions` (E9-H05).
- **Alcance**:
  - Tras `done`, escribir `.lodestar/runtime/receipts/<receiptId>.json` con `ChangeReceipt { id,
    change_set_id, previous_revision, result_revision, changed_paths, semantic_diff }`.
  - Retención: `retainReceiptsFor` (24h por defecto) y `maximumReceipts` (20); GC de los caducados/excedentes
    (borra también sus copias de recuperación).
- **Fuera de alcance**: revertir (H09).
- **Criterios de aceptación**:
  - **Dado** un apply completado, **Cuando** termina, **Entonces** existe el receipt con `previousRevision`
    y `resultRevision` correctos → `receipt_persistido`.
  - **Dado** 21 receipts con `maximumReceipts:20`, **Cuando** se hace GC, **Entonces** queda el más antiguo
    fuera y sus copias de recuperación borradas → `receipt_gc`.
- **Dependencias**: E13-H06.
- **Pruebas**: `crates/lodestar-workspace/tests/`: `receipt_persistido`, `receipt_gc`.
- **Frontera (mcp.yml)**: no.

### E13-H08 — `change_apply` (orquestación del proceso de 15 pasos)
- **Objetivo**: aplicar un plan vigente y previamente calculado, con todas las salvaguardas.
- **Referencias**: `ARCHITECTURE.md §19.5, §19.6` · `REFACTOR §11.2, §17`.
- **Alcance**:
  - Servicio `App::change_apply(changeSetId, expectedWorkspaceRevision)` que ejecuta: cargar plan →
    caducidad (`PLAN_EXPIRED`) → revisión esperada → **re-normalizar y validar** → verificar `planHash`
    (`PLAN_STALE` si difiere) → staging (H01) → validar staging → lock (H02) → re-verificar revisión →
    journal + copias (H03/H04) → publicar (H05) → reindexar → validar resultado → receipt (H07) → limpiar.
  - Tool MCP `change_apply` (perfil `standard`); política de conformidad **al arrancar** (no por llamada).
  - Salida: `{ receiptId, applied, previousWorkspaceRevision, workspaceRevision, changedPaths, semanticDiff,
    conformance{conformant,errors,warnings} }`.
- **Fuera de alcance**: revert (H09).
- **Criterios de aceptación**:
  - **Dado** un plan válido y vigente, **Cuando** se aplica, **Entonces** `applied:true` y el workspace
    queda en `resultWorkspaceRevision` → `apply_ok` (benchmark §17: "Crear un concepto válido → plan
    aceptado y aplicado").
  - **Dado** un plan cuya `planHash` ya no casa (el bundle cambió bajo él), **Cuando** se aplica, **Entonces**
    `PLAN_STALE` y no escribe → `apply_plan_stale`.
  - **Dado** un plan caducado, **Cuando** se aplica, **Entonces** `PLAN_EXPIRED` → `apply_plan_expired`.
  - **Dado** un intento de escribir fuera de `writableRoots` en las ops, **Cuando** se aplica, **Entonces**
    `PERMISSION_DENIED` y no escribe → `apply_fuera_de_writable` (benchmark §17: "Intentar escribir fuera de
    writableRoots → rechazo").
- **Dependencias**: E13-H01…E13-H07.
- **Pruebas**: `crates/lodestar-mcp/tests/`: `apply_ok`, `apply_plan_stale`, `apply_plan_expired`,
  `apply_fuera_de_writable`.
- **Frontera (mcp.yml)**: **sí**.

### E13-H09 — `change_revert` (reversión de una transacción reciente)
- **Objetivo**: revertir exclusivamente una transacción reciente y no alterada, desde sus copias de recuperación.
- **Referencias**: `ARCHITECTURE.md §19.5, §19.6` · `REFACTOR §11.3, §17`.
- **Alcance**:
  - Servicio `App::change_revert(receiptId, expectedWorkspaceRevision)` con condiciones: el receipt existe
    y no caducó; el workspace sigue en la `resultRevision`; los ficheros afectados no cambiaron; las copias
    de recuperación están; el estado restaurado se valida.
  - Aplica la restauración por el **único escritor** (nueva transacción inversa con su propio journal/receipt).
  - Tool MCP `change_revert` (perfil `standard`).
- **Fuera de alcance**: historial general (no es git; `REFACTOR §11.3`).
- **Criterios de aceptación**:
  - **Dado** un receipt reciente y el workspace intacto, **Cuando** se revierte, **Entonces** el workspace
    vuelve a `previousRevision` → `revert_reciente` (benchmark §17: "Recuperar un cambio reciente →
    change_revert").
  - **Dado** que un fichero afectado cambió tras el apply, **Cuando** se revierte, **Entonces** `WRITE_CONFLICT`
    y no revierte → `revert_fichero_alterado`.
  - **Dado** un receipt caducado/purgado, **Cuando** se revierte, **Entonces** error (no disponible) →
    `revert_caducado`.
- **Dependencias**: E13-H07, E13-H08.
- **Pruebas**: `crates/lodestar-mcp/tests/`: `revert_reciente`, `revert_fichero_alterado`, `revert_caducado`.
- **Frontera (mcp.yml)**: **sí**.

### E13-H10 — Auditoría `.lodestar/runtime/audit.jsonl`
- **Objetivo**: registrar localmente cada operación de escritura (no es conocimiento canónico).
- **Referencias**: `ARCHITECTURE.md §19.7` · `REFACTOR §14`.
- **Alcance**:
  - Anexar una línea JSON por `change_apply`/`change_revert`: `{ timestamp, client, tool, changeSetId,
    baseRevision, resultRevision, paths, result }`.
  - Runtime (gitignored, fuera de `WorkspaceRevision`, no indexado).
- **Fuera de alcance**: exponerlo por tool (es local).
- **Criterios de aceptación**:
  - **Dado** un `change_apply` exitoso, **Cuando** termina, **Entonces** `audit.jsonl` tiene una línea con
    `result:"success"` y las revisiones → `audit_registra_apply`.
  - **Dado** un apply que falla por conflicto, **Cuando** se procesa, **Entonces** la línea registra el fallo
    → `audit_registra_fallo`.
- **Dependencias**: E13-H08.
- **Pruebas**: `crates/lodestar-workspace/tests/`: `audit_registra_apply`, `audit_registra_fallo`.
- **Frontera (mcp.yml)**: no.

### E13-H11 — Auto-regeneración de `index`/`tags` dentro de `change_apply`
- **Objetivo**: mantener los generados coherentes cuando el cambio afecta la estructura, dentro de la misma transacción.
- **Referencias**: `ARCHITECTURE.md §19.6` · decisión **D6a** · `Bundle::gen_index`/`gen_tag_indexes`
  (`bundle.rs:207-208`), generadores puros (`§10` fila 12).
- **Alcance**:
  - Cuando un change set crea/mueve/borra conceptos o cambia tags, incluir en el **mismo lote** la
    regeneración de los `index.md`/índices de tags afectados (Mutation añadida al staging antes de publicar).
  - No se exponen tools MCP de generación (quedan en CLI `lodestar index`/`tags`).
- **Fuera de alcance**: regenerar en cada edición manual (eso lo cubre el watcher/CLI).
- **Criterios de aceptación**:
  - **Dado** un `create` de un concepto en un directorio con `index.md`, **Cuando** se aplica, **Entonces**
    el `index.md` regenerado incluye el nuevo concepto en el mismo receipt → `apply_regenera_index`.
  - **Dado** un cambio de tags, **Cuando** se aplica, **Entonces** los índices de tags obsoletos se purgan
    en la misma transacción → `apply_regenera_tags`.
- **Dependencias**: E13-H08.
- **Pruebas**: `crates/lodestar-workspace/tests/`: `apply_regenera_index`, `apply_regenera_tags`.
- **Frontera (mcp.yml)**: no.

---

## Orden de construcción (E13)

Estrictamente incremental por capas de la transacción: `E13-H01` (staging) → `E13-H03` (journal) →
`E13-H04` (copias) → `E13-H05` (publicar) → `E13-H06` (crash-recovery, el corazón) → `E13-H07` (receipts).
`E13-H02` (lock) es transversal y se puede construir en paralelo tras `E10-H03`, pero debe estar antes de
`E13-H08`. Con la mecánica lista, `E13-H08` (`change_apply`) la orquesta; luego `E13-H09` (`change_revert`,
necesita receipts+apply), `E13-H10` (audit) y `E13-H11` (auto-regen), enganchados a H08. **Fixtures con
`FailPoint`** (matan la publicación a mitad) son obligatorias en `E13-H06` (property `recovery_sin_parciales`)
y recomendadas en H03/H04/H05. Ninguna historia **[BLOQUEADA]**.
