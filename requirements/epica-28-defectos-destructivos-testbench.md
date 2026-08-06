# E28 — Fase 0 de la campaña de bugfixes del testbench homelab: defectos destructivos

> **Origen**: `decisiones/23-hallazgos-testbench-homelab.md` (filas M-01 y A-05, prioridades 5 y 4
> — las dos más altas del hallazgo, y las únicas de la tabla que la ficha marca como «historia
> propia, inmediata») y `docs/qa/informe-homelab-2026-08-06.md` (§1, caso G1-18, y la fila A-05 de
> §3, caso G1-11). Ninguno de los dos defectos se dedujo leyendo código: los reprodujo el testbench
> ejecutando `lodestar-mcp` por JSON-RPC sobre un worktree efímero del workspace real del homelab —
> misma disciplina que E23/E24/E25/E26.
>
> **Objetivo de la épica**: que las dos vías con **riesgo real de pérdida de conocimiento** que el
> testbench encontró en 189 casos queden cerradas: revertir un `-revert` deja de ser un no-op que
> destruye el redo, y un agente ya no puede pisar un documento existente con `create`/`move` sin que
> el plan se lo impida. Referencias maestras: `ARCHITECTURE.md §19.4/§19.5` (modelo transaccional),
> `ARCHITECTURE.md §20.11` (operaciones universales), `contracts/mcp.yml`, `CLAUDE.md` (invariantes).

**Principio rector**: el invariante #1 de `CLAUDE.md` — *«los `.md` en disco son la única fuente de
verdad»* — junto con la garantía que `ARCHITECTURE.md §19.5` promete para todo el modelo
transaccional: *«nunca un estado parcial silencioso»*. M-01 rompe esa garantía en su forma más
grave (una operación que dice haber tenido éxito y en su lugar borra la única copia de recuperación
del redo); A-05 la rompe por omisión (una operación que sí escribe, sin que nada la frenara).

**Fuera de alcance (explícito)**: **E28 NO incluye los demás hallazgos de `decisiones §23`.** En
particular:

- **D-01** (discrepancia `instructions`/`tools/list` bajo `readonly` + `protocolVersion` sin
  validar) — destino declarado: épica de honestidad de superficie.
- **D-02** (`patch_frontmatter` null-vs-remove, `ARCHITECTURE.md §20.4` vs wire RFC 7386) —
  decisión de producto abierta, no se toma aquí.
- **A-01, A-02, A-03, A-04, A-06, A-07, A-08, A-09, A-10** — huecos contractuales registrados, cada
  uno con su propio dueño (ciclo de higiene de `decisiones §16(j)`, épica de honestidad, o `§19`).
  Ninguno implica pérdida de datos ni escritura destructiva; por eso quedan fuera de esta fase.
- Cualquier repriorización de `decisiones/23-hallazgos-testbench-homelab.md` — esta épica ejecuta
  las dos filas ya priorizadas como «historia propia, inmediata»/«historia propia», no reabre la
  tabla.
- La pasada de `/mutantes` genérica sobre E25/E26 (`decisiones §16(l)`) — no forma parte de esta
  fase, aunque H01 hereda su propio arnés de mutation testing en el ciclo completo.

---

## E28-H01 — `change_revert` de un recibo `-revert` restaura de verdad, y la coreografía de sellado se unifica

- **Objetivo**: revertir un recibo `-revert` es una operación real y **componible** — restaura el
  estado que ese recibo dejó atrás, produce un recibo nuevo con identidad propia, y nunca destruye
  las copias de recuperación (`recovery/`) ni los recibos (`receipts/`) de una transacción previa.
  De paso, la secuencia de sellado duplicada entre `apply` y `revert` (`decisiones §16(i)`) se
  extrae a un único camino, para que esta clase de divergencia no pueda volver a aparecer sin que
  el compilador o la suite lo vean.

- **Síntoma reproducible** (caso G1-18, `docs/qa/testbench/batches/verify_G1-18b.json`): sobre un
  worktree limpio, `plan → apply → revert(X) → revert(X-revert)`. El segundo `revert` responde
  `reverted: true` con `changedPaths` no vacío, pero:
  - devuelve el **mismo** `receiptId` `X-revert` (el sufijo no apila);
  - `previousWorkspaceRevision == workspaceRevision` (no hay restauración real: el fichero queda
    intacto, no-op silencioso);
  - `recovery/X-revert/` —que guardaba el estado **redo**, o sea el resultado del primer `revert`—
    queda **sobrescrito** con el estado actual, y `receipts/X-revert.json` se reescribe como un
    recibo degenerado (`A→A`).

  El redo prometido por `safe-changes.md` L117-119 (*«undoing is itself a transaction, with its own
  journal and its own recovery copies»*) y por la intención de E25-H05 (`recovery.rs` L963-969:
  *«deshacer el undo» no debe ser imposible*) queda destruido de forma permanente y silenciosa, sin
  que ningún error lo señale.

- **Causa raíz** (diagnóstico ya hecho por la verificación adversarial del informe):
  `crates/lodestar-app/src/lib.rs` ~L2168, en `App::change_revert_uncounted`:
  ```rust
  let orig_txn_id = transaction_id(&receipt.change_set_id);
  let revert_txn_id = format!("{orig_txn_id}-revert");
  ```
  El recibo `X-revert` conserva el `changeSetId` **original** de la transacción que revirtió (no
  gana uno propio), así que `transaction_id(&receipt.change_set_id)` recalcula sobre `X-revert` el
  mismo `orig_txn_id = X` que calculó el primer revert. Consecuencia doble: (a)
  `revert_transaction_con_recibo` va a buscar `recovery/X/` —el árbol **pre-apply**, ya vigente y
  correcto— en vez de `recovery/X-revert/` —el árbol que guardaba el **redo**—, así que la
  restauración es un no-op sobre un estado que ya estaba ahí; (b) `revert_txn_id =
  "{orig_txn_id}-revert"` vuelve a producir literalmente `X-revert`, que es el mismo id de la
  transacción que se está revirtiendo — la nueva transacción **colisiona consigo misma** y
  sobrescribe su propio `recovery/`/`receipts/` en vez de crear una identidad nueva.

- **Referencias**: `ARCHITECTURE.md §19.4/§19.5` (modelo transaccional, recibo y copias de
  recuperación) · `contracts/mcp.yml` (`change_revert`, retorno: *«Es INVERSO al apply:
  `previousWorkspaceRevision == resultRevision` del apply revertido»* y semántica *«como una NUEVA
  transacción inversa recuperable»*) · `decisiones/16-deuda-auditoria-e25-e26.md` subpunto (i) (*«la
  secuencia de sellado está duplicada entre `apply` y `revert`»* — ver más abajo) ·
  `crates/lodestar-app/src/lib.rs` (`change_revert_uncounted`, `orig_txn_id`/`revert_txn_id` ~L2168)
  · `crates/lodestar-workspace/src/recovery.rs` (`revert_transaction_con_recibo:978`, pasos (10)/
  (10a)/(10b): promoción del recibo pendiente, borrado del journal, best-effort) ·
  `crates/lodestar-workspace/src/transaction.rs` (`apply_transaction_con_recibo:149`, pasos (11)/
  (11a)/(11b)/(11c): la misma coreografía —promover recibo, limpiar staging, borrar journal— escrita
  una segunda vez, con la única diferencia de que `apply` también limpia `staging/`) ·
  `CLAUDE.md` invariante #1 (única fuente de verdad) y #5 (único escritor).

- **Alcance**:
  - **Identidad propia del `txn_id` de cada reversión**, componible sin límite: el id de la
    transacción inversa debe derivarse de forma que revertir un `-revert` no vuelva a colisionar con
    él mismo (p. ej. numerando el sufijo o derivando el id de la transacción inversa del `receiptId`
    que se está revirtiendo, no del `changeSetId` heredado). La convención de nombre que ata
    `staging/`, `recovery/`, `journal/` y `receipts/` bajo el mismo `txnId` (documentada en
    `receipts.rs:9-15`) se preserva: lo que cambia es cómo se deriva ese `txnId`, no la convención.
  - `revert_transaction_con_recibo` debe localizar `recovery_root` sobre el **id correcto** — el
    árbol que guarda el estado anterior al recibo que efectivamente se está revirtiendo, no uno
    recalculado a partir de un `changeSetId` que varias reversiones pueden compartir.
  - **Nunca sobrescribir** `recovery/<txnId>/` ni `receipts/<txnId>.json` de una transacción cuyo
    `txnId` ya existe con contenido vigente (journal vivo o recibo persistido): si la derivación de
    id pudiera producir una colisión, debe fallar de forma ruidosa antes de escribir nada, no
    degradar en silencio a un no-op.
  - **Unificación de la coreografía de sellado** (`decisiones §16(i)`): extraer un único camino
    compartido por `apply_transaction_con_recibo` (`transaction.rs`, pasos 11/11a/11b/11c) y
    `revert_transaction_con_recibo` (`recovery.rs`, pasos 10/10a/10b) para «promover el recibo
    pendiente → limpiar staging si aplica → borrar el journal», con el mismo orden y las mismas
    garantías best-effort documentadas en ambos sitios hoy. La firma concreta (función libre,
    método de un tipo común, o lo que el análisis de la fase roja aconseje) la decide la
    implementación; el criterio de aceptación es que **no exista una segunda copia manual de esa
    secuencia** en el crate.
  - Sin cambio de forma de wire: `RevertResult` conserva sus campos (`reverted`, `receiptId`,
    `previousWorkspaceRevision`, `workspaceRevision`, `changedPaths`, `semanticDiff`).

- **Fuera de alcance**: revertir un recibo que **no** sea de una transacción `-revert` (el camino ya
  probado por la suite existente) no cambia de comportamiento; esta historia no toca el formato del
  `receiptId` de una transacción normal, solo el de sus reversiones encadenadas.

- **Criterios de aceptación**:
  - **Dado** un documento con contenido `A`, **Cuando** se hace `plan → apply` (queda en `B`) →
    `revert` (vuelve a `A`, recibo `X-revert`) → `revert` del recibo `X-revert` (debe volver a `B`),
    **Entonces** el fichero queda exactamente en el estado post-apply `B` → **test:
    `revertir_el_revert_restaura_el_estado_post_apply`**.
  - **Dado** ese mismo encadenamiento, **Cuando** se compara el `receiptId` del segundo `revert` con
    `X-revert`, **Entonces** son **distintos**, y también lo son `previousWorkspaceRevision` y
    `workspaceRevision` entre sí (hay cambio efectivo, no un no-op) → **test:
    `revertir_un_revert_produce_receipt_id_distinto`**.
  - **Dado** el árbol `recovery/X/` y `receipts/X.json` de la transacción original, y
    `recovery/X-revert/` y `receipts/X-revert.json` del primer `revert`, **Cuando** se revierte
    `X-revert`, **Entonces** los cuatro quedan **intactos byte a byte** (ni sobrescritos ni
    purgados) → **test: `revertir_un_revert_no_toca_recovery_ni_receipts_previos`**.
  - **Dado** el encadenamiento de tres reversiones (`apply` → `revert(X)` → `revert(X-revert)` →
    `revert` del resultado del paso anterior), **Cuando** se ejecuta el tercer `revert`,
    **Entonces** el fichero vuelve al estado que dejó el **primer** `revert` (composición sin
    límite, cada reversión es una operación real) → **test:
    `revertir_tres_veces_compone_sin_perder_estado`**.
  - **Dado** un crash (failpoint o `SIGKILL`) a mitad del `revert` de un `-revert`, **Cuando** se
    reabre el workspace, **Entonces** el canónico converge a uno de los dos bordes (nunca un
    parcial) y la siguiente operación funciona al primer intento, apoyándose en los failpoints ya
    existentes de `test-failpoints` si la historia lo requiere para aislar el punto de caída →
    **test: `crash_a_mitad_de_revertir_un_revert_no_deja_parciales`**.
  - **Dado** el código de `apply_transaction_con_recibo` y `revert_transaction_con_recibo` tras el
    arreglo, **Cuando** se revisa la secuencia «promover recibo → limpiar staging (si aplica) →
    borrar journal», **Entonces** existe en un único lugar compartido, no en dos copias manuales →
    revisión de diff (criterio estructural, no BDD).
  - **Dado** el camino de un `revert` sobre una transacción **normal** (no `-revert`), **Cuando** se
    ejecuta con la suite existente, **Entonces** se comporta exactamente igual que antes del arreglo
    → control anti-vacuo: los tests actuales de `revert_transaction`/`change_revert` en
    `crates/lodestar-workspace/tests/transactions.rs` y `crates/lodestar-app/tests/` siguen verdes
    sin tocarse.

- **Dependencias**: ninguna.

- **Pruebas**: arnés de sesión viva `Sesion` de
  `crates/lodestar-mcp/tests/e2e_ciclo_vida.rs` (proceso vivo: plan → apply → revert → revert
  encadenados dentro de la misma sesión JSON-RPC, que es la única forma de que un crash entre
  reversiones sea observable como lo vio el testbench) + `crates/lodestar-workspace/tests/transactions.rs`
  (unificación de la coreografía de sellado, y el caso de crash a mitad del segundo revert si se
  aísla con `--features test-failpoints`) + `crates/lodestar-app/tests/` (round-trip de revisiones y
  receiptIds). Fixtures: `lodestar-fixtures` para el workspace mínimo con un documento mutable; no
  requiere fixtures nuevos, solo la secuencia de llamadas encadenadas que el arnés ya soporta.

- **Frontera (mcp.yml)**: no cambia la forma de `change_revert` (mismos parámetros, mismo
  `RevertResult`); cambia su comportamiento interno para un caso hoy roto. No requiere delta de
  contrato.

- **Proceso**: ciclo **completo** (spec → roja → verde → juez ciego con mutation testing) — toca el
  motor transaccional y la garantía nuclear de recuperación, la misma clase de historia que
  `E24-H01`/`E24-H03` en la épica análoga.

---

## E28-H02 — `create`/`move` sobre un destino ocupado quedan bloqueados por un guard de colisión

- **Objetivo**: un `change_plan` con una operación `create` sobre un `path` ya existente, o `move`
  cuyo destino (`to`) ya está ocupado, produce un plan **no aplicable**, con un diagnóstico y un
  código de colisión declarado en el contrato — no un plan que aplica sin fricción y pisa
  conocimiento existente.

- **Síntoma reproducible** (caso G1-11, `docs/qa/testbench/batches/verify_G1-11.json`): sobre un
  workspace con un documento existente en `notas/existente.md`, un `change_plan` con
  `{"op":"create","path":"notas/existente.md", ...}` devuelve `canApply: true` sin ningún
  diagnóstico de colisión; aplicado, sobrescribiría el documento existente. Simétricamente, un
  `move` con `to` apuntando a un path ya ocupado por otro documento produce el mismo `canApply:
  true` sin fricción. El caso inverso — un `patch_frontmatter`/`replace_body`/`delete` sobre un
  `path` que **no** existe — ya está cubierto: da `DOCUMENT_NOT_FOUND`. La dirección contraria
  (crear/mover **hacia** algo que sí existe) no tiene guard.

- **Causa raíz** (verificada en el código): `crates/lodestar-core/src/plan.rs`, `normalize_create`
  (L335-346) recibe el `DocumentSet` como `_workspace` — literalmente descartado con guion bajo, sin
  comprobar si `path` ya tiene fichero — y construye el `NormalizedOperation::Create` sin condición
  alguna. `normalize_move` (L826+) tampoco consulta `doc_set.files()` para el destino `to`: solo lee
  el cuerpo de `from` (vía `document_body`, que sí falla si `from` no existe) y reescribe enlaces,
  pero nunca comprueba si `to` ya tiene contenido antes de emitir el `Move`.

- **Referencias**: `ARCHITECTURE.md §20.11` (operaciones universales del modelo de cambios) ·
  `contracts/mcp.yml` (sección `change_plan`, `operations[].op` con valores `["create", ...,
  "move", ...]`; el caso inverso `DOCUMENT_NOT_FOUND` ya declarado como *«objetivo de normalización
  inexistente»* en la lista `errores:` de `change_plan`) · `crates/lodestar-core/src/plan.rs`
  (`normalize_create:335`, `normalize_move:826`) · `crates/lodestar-app/src/lib.rs`
  (`normalize_raw_op`, brazos `"create"` ~L2596 y `"move"` ~L2675) · `CLAUDE.md` invariante #1
  (única fuente de verdad: un `create`/`move` que pisa sin avisar es la forma más directa de
  perderla).

- **Delta de contrato propuesto** (`contracts/mcp.yml`):
  - **Código nuevo**: `DOCUMENT_ALREADY_EXISTS`, añadido al catálogo `ErrorCode` de
    `lodestar-core::types` (pasaría de 16 a 17 filas — cambio consciente y declarado, igual que
    `NONCONFORMANT_RESULT → INVALID_RESULT` en E23-H14; el test
    `catalogo_de_errores_tiene_dieciseis_filas` de `crates/lodestar-core/tests/core.rs` se actualiza
    a diecisiete y su comentario documenta el motivo). Nombre elegido por simetría directa con
    `DOCUMENT_NOT_FOUND` (mismo patrón `DOCUMENT_<PREDICADO>`, mismo namespace conceptual: los dos
    describen un desajuste entre lo que la operación asume sobre la **existencia** de un `path` y lo
    que hay realmente en el `DocumentSet`) — se prefiere frente a alternativas como reusar
    `INVALID_SCHEMA` (que en el contrato está reservado a errores de **forma** de la operación, no
    de **estado** del workspace contra el que se normaliza) o `WRITE_CONFLICT` (reservado a un
    cambio de revisión entre plan y apply, no a una colisión ya visible al planificar).
  - En la fila `change_plan` de `tools:`, añadir a la lista `errores:`: `"DOCUMENT_ALREADY_EXISTS
    (create sobre un path ya ocupado por un documento existente; move cuyo «to» ya está ocupado)"`.
  - En la descripción de `operations[].op` (`change_plan`), la semántica de `create` y `move` deja
    de callar el caso: el mensaje debe nombrar el `path` colisionado, siguiendo el estilo ya fijado
    por `DOCUMENT_NOT_FOUND` (nombrar el `ref`/path que no resolvió).
  - No se toca `impact_analyze` ni `change_apply` en su forma: `change_apply` de un plan cuyo
    `canApply` ya es `false` sigue rechazado por el gate existente (E12-H04), sin necesidad de
    lógica nueva ahí — el guard vive enteramente en la normalización del plan.

- **Alcance**:
  - `normalize_create` deja de descartar `_workspace`: comprueba `doc_set.files().contains_key(path)`
    (o equivalente) y devuelve `Err(CoreError::...)` — variante nueva o reuso de una existente del
    core, mapeada por `AppError::from` a `ErrorCode::DocumentAlreadyExists` en la capa de mapeo de
    `lodestar-app` (mismo patrón que el resto de `CoreError` → `ErrorCode`) — cuando el path ya
    tiene fichero.
  - `normalize_move` comprueba `to` contra `doc_set.files()` antes de emitir el `Move`; si `to` ya
    existe, falla con el mismo código. (El caso `from` inexistente sigue siendo
    `DOCUMENT_NOT_FOUND`, vía `document_body`, sin cambios.)
  - Caso límite a decidir en la fase roja y fijar con test: `move` con `from == to` (no-op) no debe
    confundirse con una colisión — el destino coincide consigo mismo, no con un documento distinto.
  - Selección masiva (`selection`+`operation` de `change_plan`, `§Fase 12`): `create`/`move` **no**
    son ops admitidas en forma de selección masiva (el contrato ya lo declara: *«create/move no
    aplican a una selección de documentos existentes»*), así que este guard no tiene superficie que
    tocar ahí — se confirma con un criterio anti-vacuo, no con lógica nueva.

- **Fuera de alcance**: cualquier otra operación (`patch_frontmatter`, `replace_body`,
  `replace_text`, `edit_section`, `delete`) no cambia de comportamiento — su relación con la
  existencia del `path` ya está resuelta (todas exigen que el documento exista, vía
  `document_body`/`op_ref_path`). No se introduce un guard de colisión simétrico para `delete`
  (borrar algo que no existe ya es `DOCUMENT_NOT_FOUND`, dirección ya cubierta).

- **Criterios de aceptación**:
  - **Dado** un workspace con `notas/existente.md`, **Cuando** se llama a `change_plan` con
    `{"op":"create","path":"notas/existente.md"}`, **Entonces** el plan resultante tiene
    `canApply: false` y el error/diagnóstico lleva el código `DOCUMENT_ALREADY_EXISTS` nombrando el
    path → **test: `create_sobre_path_existente_es_document_already_exists`**.
  - **Dado** un workspace con `notas/existente.md` y `notas/origen.md`, **Cuando** se llama a
    `change_plan` con `{"op":"move","from":"notas/origen.md","to":"notas/existente.md"}`,
    **Entonces** el plan resultante tiene `canApply: false` y el mismo código
    `DOCUMENT_ALREADY_EXISTS` → **test: `move_a_destino_ocupado_es_document_already_exists`**.
  - **Dado** un workspace **sin** `notas/nueva.md`, **Cuando** se llama a `change_plan` con
    `{"op":"create","path":"notas/nueva.md"}`, **Entonces** el plan sigue teniendo `canApply: true`
    y se comporta exactamente igual que antes del arreglo → **test:
    `create_sobre_path_libre_sigue_funcionando`** (control anti-vacuo).
  - **Dado** un workspace con `notas/origen.md` y **sin** `notas/destino.md`, **Cuando** se llama a
    `change_plan` con `{"op":"move","from":"notas/origen.md","to":"notas/destino.md"}`,
    **Entonces** el plan sigue teniendo `canApply: true` → **test:
    `move_a_destino_libre_sigue_funcionando`** (control anti-vacuo).
  - **Dado** un plan con una colisión de `create` o `move` (`canApply: false`), **Cuando** se llama
    a `change_apply` con su `changeSetId`, **Entonces** la llamada se rechaza y **no** se escribe
    nada en disco (ni el documento colisionado ni ningún otro path del plan) → **test:
    `apply_de_plan_con_colision_rechaza_sin_tocar_disco`**.
  - **Dado** un `move` con `from == to` sobre un documento existente, **Cuando** se llama a
    `change_plan`, **Entonces** el comportamiento (no-op válido, o el que la fase roja fije con
    test) queda declarado — no se confunde con una colisión de destino distinto → **test:
    `move_a_si_mismo_no_es_colision`**.
  - **Dado** el catálogo de `ErrorCode`, **Cuando** se cuenta tras el arreglo, **Entonces** tiene
    **17** filas (la nueva incluida) y el test que lo fija está actualizado a conciencia → revisión
    de diff sobre `catalogo_de_errores_tiene_dieciseis_filas` (pasa a
    `catalogo_de_errores_tiene_diecisiete_filas`, o el nombre que la implementación fije).

- **Dependencias**: ninguna. Es implementable sin **E28-H01**, y viceversa — las dos historias tocan
  ficheros distintos (`recovery.rs`/`transaction.rs` vs `plan.rs`) y garantías distintas (nunca
  destruir un redo vs nunca pisar en silencio); H01 va primero en el orden de construcción por
  gravedad (pérdida de datos ya escrita) y prioridad (`5` vs `4` en `decisiones §23`), no por
  bloqueo técnico.

- **Pruebas**: `roundtrip()` de `crates/lodestar-mcp/tests/mcp.rs` (arnés de proceso frío,
  suficiente porque cada caso es una llamada aislada `change_plan`/`change_apply` sin estado de
  sesión que mantener entre llamadas) + `crates/lodestar-core/tests/core.rs` (normalización pura de
  `normalize_create`/`normalize_move`, incluida la actualización del test del catálogo de
  `ErrorCode`) + `crates/lodestar-app/tests/plan.rs` (mapeo `CoreError` → `AppError` con el código
  nuevo). Fixtures: `lodestar-fixtures`, workspace mínimo con al menos dos documentos existentes
  para poder ejercer la colisión de `move`.

- **Frontera (mcp.yml)**: **sí** — nueva fila en el catálogo `ErrorCode` (16→17) y nueva entrada en
  la lista `errores:` de `change_plan`. El delta queda descrito arriba, en la sección «Delta de
  contrato propuesto»; se aplica en la misma pasada que el código, no como seguimiento separado.

- **Proceso**: ciclo **completo** — abre el catálogo de errores congelado (cambio consciente de
  wire, la misma clase de decisión que motivó el test `catalogo_de_errores_tiene_dieciseis_filas`)
  y toca la forma de `change_plan`; no es un defecto puramente acotado.

---

## Orden de construcción

```
H01 (revert de un -revert + unificación del sellado)   ┐  independientes entre sí,
H02 (guard de colisión create/move)                     ┘  paralelizables

H01 va primero en la secuencia de trabajo por gravedad (pérdida de datos activa, prioridad 5
en decisiones §23) y por compartir zona con la deuda ya decidida de §16(i); H02 es la segunda
historia de la fase 0, según fija decisiones §23 («M-01 entra por delante de la épica de
honestidad… A-05 entra como segunda historia»).
```

## Criterio de salida

Revertir un recibo `-revert` restaura el estado real que ese recibo dejó atrás, con receiptId e
identidad propios, sin sobrescribir jamás `recovery/`/`receipts/` de una transacción previa, y la
coreografía de sellado de `apply`/`revert` vive en un único camino compartido. Un `create` sobre un
path existente o un `move` a un destino ocupado producen un plan no aplicable con un código de
colisión declarado en el contrato, y su `change_apply` queda rechazado sin tocar disco. Las dos
historias tienen su ciclo TDD completo con juez ciego y mutation testing, dado que ambas tocan el
motor transaccional o la forma congelada del catálogo de errores.

---

## Adenda correctiva (2026-08-06)

Los jueces ciegos que revisaron la implementación de H01 y H02 (commits `043f233` y `296147b`, ya
en `develop`) verificaron cada veredicto **ejecutando el binario real** por JSON-RPC — la misma
disciplina que produjo la épica — y encontraron dos defectos reales, cada uno un bloqueante de la
historia que arregló: H01 dejó viva una segunda vía de colisión de `txnId` (el `apply`, no el
`revert`, que sí quedó protegido) y H02 normaliza contra el `DocumentSet` **inicial** del plan en
vez del estado que las propias operaciones del plan van acumulando, lo que produce tanto falsos
negativos destructivos como una regresión de dos idiomas legítimos que funcionaban antes de H02.
**E28-H03** cierra el primero; **E28-H04** cierra el segundo. Los hallazgos no se relitigan aquí:
se especifican como corrección.

---

## E28-H03 — identidad de transacción libre en la publicación (corrige el bloqueante de H01)

- **Objetivo**: la identidad efectiva de **toda** transacción de publicación —`change_apply` y
  `change_revert` por igual— se resuelve buscando de forma determinista la primera variante libre
  del `txnId`, con el mismo criterio de «libre» que ya usa el guard de H01 y el GC del plano de
  control (`journal/` ∪ `receipts/`), en vez de sobrescribir en silencio (el camino que hoy toma
  `change_apply`) o fallar sin salida (el camino que hoy toma `change_revert` cuando el `txnId` de
  su propia reversión ya tiene recibo vigente por culpa de ese primer defecto).

- **Síntoma reproducible** (verificado por JSON-RPC contra el binario real): el `changeSetId` es
  determinista — `blake3(baseRevision, normalizedOperations)` (`crates/lodestar-app/src/lib.rs`
  ~L1792, `compute_plan_hash`) — así que replanificar **exactamente** el mismo cambio sobre la
  misma base produce el mismo `changeSetId`, y por tanto el mismo `txnId`
  (`transaction_id(&change_set.id)`, `crates/lodestar-workspace/src/transaction.rs:68`). La
  secuencia `plan → apply(X) → revert(X) → re-plan idéntico → apply(X)` reutiliza literalmente el
  mismo `txnId` `X` que la primera transacción:
  - el segundo `apply` **sobrescribe** `recovery/X/` y `receipts/X.json` de la primera
    transacción, porque `apply_transaction_con_recibo` (`transaction.rs:280`) llama a
    `backup_originals`/`create_journal`/`write_pending_receipt` directamente, sin pasar antes por
    el guard `assert_txn_id_libre` que H01 escribió (`crates/lodestar-workspace/src/recovery.rs:912`)
    — ese guard **solo** lo llama `revert_transaction` (`recovery.rs:1086`), nunca
    `apply_transaction_con_recibo`;
  - el `revert(X)` posterior a ese segundo `apply` falla `WRITE_CONFLICT` **sin salida**: el
    `revert_transaction_id` que deriva su propio `txnId` (`X-revert`) encuentra que `X-revert` ya
    tiene un recibo persistido — el de la **primera** reversión, la que el paso anterior de la
    secuencia ya había hecho — y `assert_txn_id_libre` lo rechaza correctamente, pero no hay ningún
    id alternativo que probar: el re-apply queda **permanentemente no revertible**.

  En el commit padre a H01 (antes de que existiera el guard) esa misma secuencia funcionaba —sin
  garantía real de no-colisión, pero sin bloquear—; H01 cerró el camino del `revert` con un `Err`
  ruidoso y dejó abierto el mismo camino en el `apply`, y ahora la combinación de los dos (uno que
  sobrescribe, otro que rechaza) dejó una secuencia legítima sin salida.

- **Causa raíz**: `assert_txn_id_libre` (`crates/lodestar-workspace/src/recovery.rs:912`) es un
  guard **rechaza-o-nada**: comprueba si el `txnId` propuesto está libre y falla si no lo está, pero
  no participa en decidir **cuál** `txnId` usar. `revert_transaction_id` (`transaction.rs:108`) sí
  deriva un `txnId` distinto en cada escalón de la cadena de reversiones (`X-revert`,
  `X-revert-2`, …), pero solo para la dirección "revertir un `-revert`"; no existe una derivación
  equivalente para "publicar de nuevo bajo un `txnId` que ya está tomado por una transacción
  **distinta** que compartía el mismo `changeSetId`" — el caso de un `apply` re-planificado
  idéntico.

- **Referencias**: `ARCHITECTURE.md §19.4/§19.5` (modelo transaccional, recibo, copias de
  recuperación) · `ARCHITECTURE.md §20.11` (operaciones universales) · `contracts/mcp.yml`
  (`change_apply`/`change_revert`) · `crates/lodestar-workspace/src/transaction.rs`
  (`transaction_id:68`, `revert_transaction_id:108`, `apply_transaction_con_recibo:280`,
  `seal_published_transaction:177`) · `crates/lodestar-workspace/src/recovery.rs`
  (`assert_txn_id_libre:912`, `revert_transaction:994`, `revert_transaction_con_recibo`) ·
  `crates/lodestar-app/src/lib.rs` (`compute_plan_hash` ~L1792, `App::change_revert_uncounted`) ·
  `CLAUDE.md` invariante #1 (única fuente de verdad: un recibo pisado es una copia de recuperación
  perdida) y #5 (único escritor).

- **Alcance**:
  - Un único punto de decisión de identidad, consumido por **ambos** caminos (`apply` y `revert`):
    dado un `txnId` candidato, si `assert_txn_id_libre` lo rechaza, deriva la siguiente variante
    determinista (la misma familia de sufijo numerado que ya usa `revert_transaction_id` para la
    cadena de reversiones) y reintenta, hasta encontrar la primera libre. Determinista: la misma
    secuencia de entrada produce siempre el mismo `txnId` final, para que un reintento tras crash
    converja al mismo material que la recuperación ya conoce (mismo principio que ya documenta
    `revert_transaction_id`).
  - `apply_transaction_con_recibo` pasa por este punto de decisión **antes** de
    `backup_originals`/`create_journal` (el mismo lugar donde hoy calcula `txn_id =
    transaction_id(&change_set.id)`, `transaction.rs:329`), así que un `apply` cuyo `txnId`
    "natural" ya está tomado por una transacción vigente (journal presente o recibo persistido) no
    lo sobrescribe: publica bajo la primera variante libre.
  - `revert_transaction`/`revert_transaction_con_recibo` consumen el mismo punto de decisión en vez
    de un `assert_txn_id_libre` de solo-rechazo: si el `txnId` que `revert_transaction_id` deriva
    ya está tomado (el caso descrito arriba), no falla sin salida — encuentra la siguiente variante
    libre de la misma familia y revierte bajo ese id.
  - La convención de nombre que ata `staging/`, `recovery/`, `journal/` y `receipts/` bajo el mismo
    `txnId` (`receipts.rs:9-15`) se preserva sin cambios: lo que cambia es únicamente cómo se
    resuelve el `txnId` efectivo antes de la primera escritura, nunca la convención en sí. Los
    recibos y copias de recuperación **previos** (de cualquier transacción con material vigente)
    jamás se pisan, en ninguno de los dos caminos.

- **Fuera de alcance**: no cambia la forma de wire de `change_apply`/`change_revert` (mismos
  parámetros, mismos `ApplyResult`/`RevertResult`); esta historia resuelve identidad **interna**, no
  la superficie. No toca `change_plan` ni el `planHash`: dos planes idénticos sobre la misma base
  siguen produciendo el mismo `changeSetId` — eso es determinismo deseado, no el defecto.

- **Criterios de aceptación**:
  - **Dado** un documento en estado `A`, **Cuando** se ejecuta `plan → apply → revert → re-plan
    idéntico (misma base, mismas ops) → apply → revert`, **Entonces** las cuatro operaciones
    (apply, revert, apply, revert) completan con éxito, cada una con un `receiptId` **distinto** de
    los tres anteriores, y el fichero queda en el estado correcto tras cada paso → **test:
    `apply_revert_reapply_revert_de_plan_identico_completa_con_cuatro_receipts_unicos`**.
  - **Dado** ese mismo encadenamiento, **Cuando** se inspecciona `recovery/`/`receipts/` tras el
    segundo `apply`, **Entonces** las copias y el recibo de la **primera** transacción
    (`recovery/X/`, `receipts/X.json`) siguen **intactos byte a byte** — el segundo `apply` publicó
    bajo un `txnId` distinto → **test: `reapply_de_changeset_identico_no_pisa_recovery_ni_receipts_previos`**.
  - **Dado** el camino de publicación **sin** colisión de `txnId` (el caso normal: cada
    `changeSetId` es nuevo), **Cuando** se ejecuta la suite existente, **Entonces** los ids
    derivados son exactamente los de hoy (`X`, `X-revert`, `X-revert-2`, …) → control anti-vacuo:
    los tests actuales de `transaction_id`/`revert_transaction_id` y de
    `crates/lodestar-workspace/tests/transactions.rs` siguen verdes sin tocarse.
  - **Dado** el catálogo de casos del rustdoc de `revert_transaction_id` (la tabla `txn_id` →
    `reversión` de `transaction.rs:91-95`), **Cuando** se ejercen como tests unitarios uno a uno,
    **Entonces** cada fila de la tabla tiene su test → **test:
    `revert_transaction_id_sigue_la_tabla_del_rustdoc`** (parametrizado o uno por fila).
  - **Dado** un `txn_id` en su borde `-revert-{u64::MAX}`, **Cuando** se deriva su reversión,
    **Entonces** el comportamiento queda decidido y documentado explícitamente en el rustdoc (hoy es
    un punto fijo silencioso por `saturating_add`: la siguiente reversión produciría el mismo id y
    volvería a colisionar) — la fase roja fija si el borde falla ruidosamente o si se acepta el
    punto fijo con una nota que explique por qué es inofensivo en la práctica → **test:
    `revert_transaction_id_en_el_borde_u64_max`**.
  - **Dado** un `txn_id` con un sufijo **no canónico** (`-revert-1` con cero relleno-cero implícito
    ya cubierto, `-revert-+2`, `-revert-01`), **Cuando** se deriva su reversión, **Entonces** el
    resultado es el que la fase roja fije con test — cada entrada de esta clase es una decisión
    explícita, no un comportamiento accidental de `parse::<u64>()` → **test:
    `revert_transaction_id_con_sufijos_no_canonicos`**.
  - **Dado** el rustdoc de `revert_transaction_id` (*"componible sin límite"*), **Cuando** se lee
    tras el arreglo, **Entonces** reconoce explícitamente el borde de `u64::MAX` en vez de afirmar
    composición ilimitada sin matiz → revisión de diff (criterio estructural, no BDD).

- **Delta de contrato** (`contracts/mcp.yml`):
  - La fila `change_revert` debe declarar, en su lista `errores:` de `WRITE_CONFLICT`, la causa
    nueva que introdujo H01: *«el `txnId` derivado para la reversión ya identifica a una
    transacción con material vigente (journal o recibo)»*. **Si esta historia hace que esa causa
    desaparezca del camino feliz** (porque ahora se resuelve buscando la variante libre en vez de
    fallar), la entrada de `errores:` se retira en la misma pasada y se declara, en su lugar, la
    causa de `WRITE_CONFLICT` que quede vigente (ninguna, salvo las ya existentes de revisión
    optimista) — la implementación decide cuál de las dos redacciones aplica y la fase roja lo fija
    con test antes de tocar el contrato.
  - Igual para `change_apply`: si `apply_transaction_con_recibo` empieza a resolver identidad en
    vez de sobrescribir, su lista `errores:` no gana ningún código nuevo (el defecto no producía un
    error, producía una sobrescritura silenciosa) — no hay entrada que añadir ahí, solo el
    comportamiento corregido.
  - El mensaje de error de cualquier `WRITE_CONFLICT` que sobreviva debe ser **accionable para un
    agente** (qué hacer: replanificar, no rutas internas de `recovery/`/`receipts/` que un agente
    no puede interpretar ni actuar), siguiendo el estilo ya fijado por el resto del catálogo
    (`ARCHITECTURE.md §20`, principio de E26).

- **Nota de registro sin acción** (no requiere trabajo, se documenta para que no se "unifique" por
  inercia): `finish_recovery_completada` (`crates/lodestar-workspace/src/recovery.rs` ~L802)
  comparte forma superficial con `seal_published_transaction` (la coreografía unificada de H01:
  promover recibo → limpiar staging → borrar journal) pero es la vía **COMPLETAR** post-crash de
  `Workspace::recover`, con semántica deliberadamente distinta: borrado incondicional del journal
  (no condicionado a que el recibo quedara a salvo, como sí lo está `seal_published_transaction`),
  promoción idempotente (E25-H04) y ejecutándose fuera de la ventana de publicación normal. Dos
  jueces ciegos de la ronda de E28 la verificaron como legítimamente distinta de la coreografía
  compartida. Queda documentado en el rustdoc de `finish_recovery_completada` para que nadie la
  fusione con `seal_published_transaction` sin releer por qué son dos caminos.

- **Dependencias**: ninguna (corrige H01, que ya está integrada en `develop`).

- **Pruebas**: `crates/lodestar-workspace/tests/transactions.rs` (unicidad de `txnId` bajo
  colisión, control anti-vacuo de la convención existente) + tests unitarios de
  `crates/lodestar-workspace/src/transaction.rs` (tabla de casos de `revert_transaction_id` contra
  su rustdoc, borde `u64::MAX`, sufijos no canónicos) + arnés de sesión viva `Sesion` de
  `crates/lodestar-mcp/tests/e2e_ciclo_vida.rs` (la secuencia completa
  `plan→apply→revert→re-plan→apply→revert` por JSON-RPC, que es como el testbench la reprodujo).
  Fixtures: `lodestar-fixtures`, workspace mínimo con un documento mutable; no requiere fixtures
  nuevos.

- **Proceso**: ciclo **completo** (spec → roja → verde → juez ciego con mutation testing) — toca el
  mismo motor transaccional que H01 y corrige un bloqueante que dos jueces ciegos ya verificaron
  ejecutando el binario.

---

## E28-H04 — normalización contra el estado acumulado del change set (corrige el bloqueante de H02)

- **Objetivo**: `change_plan` normaliza cada operación de un plan contra el estado que las
  operaciones **anteriores del mismo plan** van dejando, no contra el `DocumentSet` con el que
  empezó a planificar. Los idiomas legítimos que dependen de eso (mover dos documentos al mismo
  destino final en dos pasos, crear un documento y moverlo en el mismo plan) vuelven a funcionar
  exactamente como antes de H02, y las colisiones reales **dentro** de un mismo plan (dos `create`
  al mismo path, un `create` que pisa un `move` anterior del propio plan) quedan rechazadas con el
  mismo `DOCUMENT_ALREADY_EXISTS` que H02 introdujo para las colisiones contra disco.

- **Síntoma reproducible** (verificado por JSON-RPC contra el binario real, sobre el commit de
  H02): `change_plan` normaliza **todas** las operaciones del array contra el `DocumentSet` inicial
  del workspace (`crates/lodestar-app/src/lib.rs` ~L1763, el bucle que llama a
  `normalize_raw_op(&doc_set, raw)` con el mismo `doc_set` en cada iteración; los guards de
  colisión que H02 añadió viven en `crates/lodestar-core/src/plan.rs`
  `normalize_create` ~L346 y `normalize_move` ~L848, y los dos comparan contra ese mismo
  `DocumentSet` fijo). Dos familias de defecto, verificadas ambas:
  - **Falsos negativos destructivos** (el guard de H02 no ve la colisión):
    - `[move a→final, move b→final]`: cada `move` se normaliza contra el `DocumentSet` original,
      donde `final` no existe todavía — el plan entero aplica con `risk: low` y el segundo `move`
      destruye lo que dejó el primero, sin ningún diagnóstico.
    - `[create X, move b→X]`: el `move` se normaliza contra el `DocumentSet` original, donde `X`
      tampoco existe — el `create` de `X` queda pisado en silencio por el `move`.
    - `[create X, create X]`: el segundo `create` se normaliza contra el mismo `DocumentSet`
      original que el primero — ninguno de los dos ve al otro, y el segundo gana al aplicar.
  - **Falsos positivos — regresión respecto al commit padre de H02** (el guard rechaza algo que
    antes funcionaba y sigue siendo legítimo):
    - `[delete X, create X]` — recrear un documento borrado en el mismo plan — funcionaba en el
      commit anterior a H02 y ahora `normalize_create` ve `X` todavía presente en el `DocumentSet`
      inicial (el `delete` previo del mismo plan no se ha reflejado) y rechaza
      `DOCUMENT_ALREADY_EXISTS`.
    - `[move A→B, create A]` — liberar `A` moviéndolo y reutilizar el path en el mismo plan —
      funcionaba antes de H02 y ahora falla igual, por la misma razón: el `DocumentSet` inicial
      todavía tiene `A` ocupado por sí mismo en el momento en que se normaliza el `create`.

- **Causa raíz**: el bucle de normalización de `App::change_plan_uncounted`
  (`crates/lodestar-app/src/lib.rs` ~L1741-1764) pasa el **mismo** `&doc_set` —inmutable, calculado
  una vez al principio de la función— a cada llamada de `normalize_raw_op`, así que ninguna
  operación ve el efecto de las que la preceden dentro del propio plan. Los guards de colisión de
  H02 (`plan.rs` `normalize_create:346`, `normalize_move:847`) son correctos **para el caso de una
  sola operación contra disco**; el defecto no está en el guard, está en qué `DocumentSet` se le
  pasa cuando el plan tiene más de una operación tocando paths relacionados.

- **Referencias**: `ARCHITECTURE.md §19.4` (`change_plan`, normalización pura sin escribir) ·
  `ARCHITECTURE.md §20.11` (operaciones universales) · `contracts/mcp.yml` (`change_plan`,
  catálogo `ErrorCode`, `DOCUMENT_ALREADY_EXISTS` introducido por H02) ·
  `crates/lodestar-app/src/lib.rs` (`App::change_plan_uncounted` ~L1700-1770, `single_operation`
  ~L2899-2924) · `crates/lodestar-core/src/plan.rs` (`normalize_create:340`, `normalize_move:839`) ·
  `CLAUDE.md` invariante #1 (única fuente de verdad) y #3 (una sola verdad computada: el estado
  contra el que se valida un plan es el que el plan mismo va construyendo, no un snapshot que deja
  de ser cierto a la segunda operación).

- **Alcance**:
  - La normalización del array de operaciones lleva un **estado de ocupación acumulado**,
    derivado del `DocumentSet` inicial y actualizado en orden por cada operación ya normalizada de
    ese mismo plan: cada `create`/`move.to` que se acepta **ocupa** su path en ese estado; cada
    `delete`/`move.from` que se acepta **libera** el suyo. Los guards de `normalize_create`/
    `normalize_move` consultan ese estado acumulado, no el `DocumentSet` fijo original.
  - Forma concreta a decidir en la fase roja (no la fija la historia, la fija el análisis): puede
    ser un `DocumentSet` hipotético que se recalcula tras cada operación aceptada (más simple,
    reusa `plan::apply_normalized_ops` incrementalmente) o un conjunto de paths ocupados/liberados
    llevado aparte junto al `DocumentSet` original (más barato, evita recomputar contenido). El
    criterio de aceptación es el comportamiento observable de los cinco escenarios, no la forma
    interna.
  - Las tres colisiones **intra-plan** (`[move a→final, move b→final]`, `[create X, move b→X]`,
    `[create X, create X]`) deben fallar con `DOCUMENT_ALREADY_EXISTS`, nombrando el `path`
    colisionado, en la operación que lo detecta (la segunda de cada par).
  - Los dos idiomas legítimos (`[delete X, create X]`, `[move A→B, create A]`) deben volver a
    aplicar, con el disco final coincidiendo exactamente con lo que producían antes de H02.
  - La selección masiva (`selection`+`operation`, expandida por `expand_selection`) queda **fuera**
    de este acumulado: sigue expandiéndose contra el `DocumentSet` inicial en un solo paso —cada
    documento seleccionado genera como mucho una operación, y `create`/`move` ya están excluidos de
    esa vía por `single_operation`— así que no hay secuencia intra-selección que acumular.

- **Fuera de alcance**: equivalencia de paths por mayúscula/minúscula o forma de normalización
  Unicode (NFC/NFD) — el guard de colisión (de H02 y de esta historia) sigue comparando claves
  **byte a byte**, sin normalizar. Ver `decisiones/24-equivalencia-caja-unicode.md` (nueva, estado
  `abierta`): en sistemas de ficheros case-insensitive (macOS/Windows) un `create`/`move` a
  `Notas/Existente.md` evade el guard byte-a-byte y puede destruir `notas/existente.md` en disco, y
  esta historia no lo corrige — necesita una decisión de producto sobre qué clave usar para
  comparar, no una corrección acotada.

- **Criterios de aceptación**:
  - **Dado** un workspace con `a.md` y `b.md` y sin `final.md`, **Cuando** se llama a `change_plan`
    con `[{"op":"move","from":"a.md","to":"final.md"},
    {"op":"move","from":"b.md","to":"final.md"}]`, **Entonces** el plan tiene `canApply: false` con
    `DOCUMENT_ALREADY_EXISTS` nombrando `final.md` en la segunda operación → **test:
    `dos_moves_al_mismo_destino_en_el_mismo_plan_es_document_already_exists`**.
  - **Dado** un workspace con `b.md` y sin `x.md`, **Cuando** se llama a `change_plan` con
    `[{"op":"create","path":"x.md",...}, {"op":"move","from":"b.md","to":"x.md"}]`, **Entonces** el
    plan tiene `canApply: false` con `DOCUMENT_ALREADY_EXISTS` nombrando `x.md` → **test:
    `create_seguido_de_move_al_mismo_path_es_document_already_exists`**.
  - **Dado** un workspace sin `x.md`, **Cuando** se llama a `change_plan` con
    `[{"op":"create","path":"x.md",...}, {"op":"create","path":"x.md",...}]`, **Entonces** el plan
    tiene `canApply: false` con `DOCUMENT_ALREADY_EXISTS` nombrando `x.md` en la segunda operación
    → **test: `dos_creates_al_mismo_path_en_el_mismo_plan_es_document_already_exists`**.
  - **Dado** un workspace con `x.md`, **Cuando** se llama a `change_plan` con
    `[{"op":"delete","path":"x.md"}, {"op":"create","path":"x.md",...}]`, **Entonces** el plan
    tiene `canApply: true`, y `change_apply` sobre ese `changeSetId` deja `x.md` en disco con el
    contenido del segundo `create` → **test: `delete_seguido_de_create_del_mismo_path_aplica`**.
  - **Dado** un workspace con `A.md` y sin `B.md`, **Cuando** se llama a `change_plan` con
    `[{"op":"move","from":"A.md","to":"B.md"}, {"op":"create","path":"A.md",...}]`, **Entonces** el
    plan tiene `canApply: true`, y `change_apply` deja `B.md` con el contenido movido y `A.md` con
    el contenido del `create` → **test: `move_seguido_de_create_del_path_liberado_aplica`**.
  - **Dado** un workspace con `notas/existente.md` (control anti-vacuo del criterio original de
    H02), **Cuando** se llama a `change_plan` con una sola operación `{"op":"create",
    "path":"notas/existente.md"}`, **Entonces** sigue rechazando `DOCUMENT_ALREADY_EXISTS` —la
    colisión contra disco de una sola operación no se rompe al introducir el acumulado → control
    anti-vacuo: `create_sobre_path_existente_es_document_already_exists` de H02 sigue verde sin
    tocarse.
  - **Dado** una selección masiva (`selection`+`operation`) que pida `{"create": {...}}` o
    `{"move": {...}}`, **Cuando** se llama a `change_plan`, **Entonces** se rechaza
    `INVALID_SCHEMA` con el mensaje de `single_operation` (*«create» no aplica a documentos
    existentes y «move» necesita un destino por documento*) → **test:
    `seleccion_masiva_rechaza_create_y_move`** (criterio pendiente de H02, cerrado aquí; vive en
    `crates/lodestar-app/tests/seleccion.rs`).

- **Limpieza de fósiles** (localizados por los jueces, en la misma pasada de esta historia, no como
  seguimiento separado):
  - `crates/lodestar-mcp/tests/mcp.rs:6480` — el comentario afirma «El catálogo NO se toca: sigue
    teniendo 16 filas (`catalogo_de_errores_tiene_dieciseis_filas`…)», y H02 ya lo llevó a 17 y
    renombró el test. Corregir el comentario para que cite el estado actual (17 filas, nombre del
    test vigente).
  - `crates/lodestar-mcp/tests/descubribilidad.rs:1921` — «de las 16 filas» en el rustdoc de la
    auditoría `codigos_sin_emisor`/`E26-H11`: actualizar a 17.
  - `contracts/mcp.yml:100` — «El catálogo sigue teniendo 16 filas: se sustituyó una» (nota
    histórica de `NONCONFORMANT_RESULT → INVALID_RESULT`, E23-H14): sigue siendo cierto **como
    afirmación histórica** de esa migración, pero en presente induce a error ahora que H02 abrió el
    catálogo a 17; reformular para que quede claro que describe el estado en E23-H14, no el actual.
  - `CHANGELOG.md`, sección `[No publicado]`: añadir una entrada bajo un encabezado `### Añadido` (o
    el que corresponda al estilo de la sección) documentando `DOCUMENT_ALREADY_EXISTS` como código
    nuevo del catálogo (16→17) y su motivo, siguiendo el estilo de las entradas ya presentes en esa
    sección.

- **Fuera de alcance explícito**: equivalencia de paths por caja/Unicode → `decisiones
  §24` (ficha nueva). Las demás operaciones (`patch_frontmatter`, `replace_body`, `replace_text`,
  `edit_section`, `delete`) no ganan un guard de colisión nuevo en esta historia — su relación con
  la **existencia** de un path no cambia (siguen exigiendo, vía `document_body`/`op_ref_path`, que
  el documento exista en el `DocumentSet` contra el que se normalizan). **Corrección (2026-08-06,
  re-jueces ciegos)**: la frase de una versión anterior de esta historia afirmaba que esas
  operaciones «ya operan sobre el cuerpo tras la normalización de la operación anterior sin el
  defecto descrito aquí» — **es falsa**, verificada contra el binario real en tres commits: `[delete
  A, patch_frontmatter A]` resucita `A.md` con solo el frontmatter nuevo y sin el cuerpo original;
  `[delete A, replace_body A]` lo resucita con el cuerpo nuevo; `[move A→B, replace_text A]` deja
  **dos** documentos vivos (`B.md` con el contenido movido y `A.md` resucitado con el resultado del
  `replace_text`). Es un defecto **preexistente** — idéntico antes y después de H02, no una
  regresión de esta historia — porque esas operaciones juzgan existencia y contenido contra el
  `DocumentSet` **inicial** del plan, exactamente la misma vía de estado obsoleto que el resto de
  esta historia corrige para `create`/`move`, y por tanto pueden **resucitar** un path que una
  operación anterior del mismo plan liberó (`delete`/`move.from`). Queda fuera de esta historia a
  propósito — el acumulado que H04 introduce es de **ocupación de path** (para los guards de
  colisión de `create`/`move`), no de **contenido acumulado** (lo que exigiría que
  `document_body`/`op_ref_path` resuelvan contra el resultado de las operaciones previas del plan,
  un cambio de forma distinto y más amplio) — y se registra como hallazgo de seguimiento en la
  adenda final de esta épica.

  **Ampliación (2026-08-06, re-jueces ciegos): la familia de move-chains preexistentes es mayor que
  un solo caso.** El texto original de esta historia nombraba únicamente `[create X, move X→Y]` →
  `NORMALIZE_TARGET_NOT_FOUND` como caso preexistente fuera de alcance (el `create` de `X` no está
  reflejado en el `DocumentSet` inicial cuando se normaliza el `move` que lo usa como `from`). La
  verificación de los re-jueces confirma que esa es solo una instancia de una familia más amplia:
  **cualquier cadena donde el origen (`from`) de un `move`, o el path que una operación de contenido
  referencia, fue producido por una operación anterior del mismo plan** comparte la misma causa —
  el cuerpo/existencia se resuelve contra el `doc_set` inicial, no contra el estado acumulado.
  Instancias adicionales confirmadas: `[move A→B, move B→C]` (el segundo `move` no ve que `B` ya
  tiene el contenido movido por el primero: falla o usa contenido obsoleto según el estado inicial
  de `B`); el swap `A↔B` vía un path temporal (`[move A→tmp, move B→A, move tmp→B]`, donde el
  tercer paso depende del primero). Distinción importante: esta familia es sobre la **ocupación del
  ORIGEN** (`from` de un `move`, o el path referenciado por una op de contenido) y su contenido, no
  sobre la ocupación del **DESTINO** (`to` de un `create`/`move`), que es exactamente lo que el
  acumulado de esta historia sí resuelve. Ambas familias — resurrección por ops de contenido y
  move-chains por origen — quedan fuera de esta historia y registradas como hallazgo de seguimiento
  en la adenda final de esta épica.

- **Dependencias**: ninguna (corrige H02, que ya está integrada en `develop`). Independiente de
  **E28-H03**: ficheros distintos (`lib.rs`/`plan.rs` de `change_plan` vs
  `transaction.rs`/`recovery.rs` de `change_apply`/`change_revert`), garantías distintas.

- **Pruebas**: `roundtrip()` de `crates/lodestar-mcp/tests/mcp.rs` (arnés de proceso frío, cada
  escenario es una llamada aislada `change_plan`/`change_apply`) + `crates/lodestar-core/tests/core.rs`
  (normalización pura con estado acumulado, si la forma elegida en la fase roja vive en el core) +
  `crates/lodestar-app/tests/plan.rs` (los cinco escenarios BDD, si la forma elegida vive en la
  fachada) + `crates/lodestar-app/tests/seleccion.rs` (el criterio anti-vacuo de selección masiva
  pendiente de H02). Fixtures: `lodestar-fixtures`, workspace mínimo con al menos dos documentos
  existentes para poder ejercer las colisiones intra-plan.

- **Frontera (mcp.yml)**: no cambia la forma de `change_plan` (mismos parámetros, mismo
  `PlanResult`, mismo catálogo `ErrorCode` de 17 filas que H02 ya fijó); cambia el comportamiento
  interno de la normalización. No requiere delta de contrato nuevo, solo la limpieza de fósiles ya
  listada (que corrige texto ya desactualizado, no introduce superficie nueva).

- **Proceso**: ciclo **completo** — toca la normalización pura del core/app que H02 dejó con un
  bloqueante verificado por jueces ciegos, y siete criterios de aceptación con forma BDD que
  necesitan roja→verde con separación de poderes.

---

## Orden de construcción (adenda)

```
H03 (identidad de txn libre en apply/revert)   ┐  independientes entre sí,
H04 (normalización acumulada create/move)       ┘  paralelizables

Las dos corrigen bloqueantes de historias ya integradas (H01 y H02 respectivamente) y no comparten
fichero ni garantía; el orden entre ellas es indiferente. Ambas preceden a cualquier trabajo que
dependa de que change_apply/change_revert/change_plan estén libres de estos dos defectos —en
particular, cualquier arranque de la épica de honestidad de superficie (E29) que ejercite el camino
transaccional debe asumir H03/H04 ya integradas.
```

## Criterio de salida (adenda)

Una secuencia `apply → revert → re-apply idéntico → revert` completa de punta a punta con cuatro
`receiptId` únicos, sin pisar jamás `recovery/`/`receipts/` de una transacción previa, en **ambos**
caminos (`change_apply` y `change_revert`). Un plan con `[move a→final, move b→final]`,
`[create X, move b→X]` o `[create X, create X]` queda `canApply: false` con
`DOCUMENT_ALREADY_EXISTS`, y `[delete X, create X]`/`[move A→B, create A]` vuelven a aplicar con el
disco final correcto. Los cuatro fósiles de comentarios/changelog que citaban el catálogo de 16
filas quedan corregidos. Las dos historias tienen su ciclo TDD completo con juez ciego y mutation
testing, por tocar el mismo motor transaccional y la misma normalización pura que sus historias
padre.

---

## Hallazgos preexistentes registrados (2026-08-06, re-jueces)

Durante la verificación ciega de H04 aparecieron dos hallazgos que **no** son bloqueantes de esta
adenda (no regresionan nada que H01–H04 prometan) pero comparten causa raíz con lo que H04 corrige
y quedan fuera de su alcance a propósito. Se registran aquí para que no se pierdan ni se relitiguen
sueltos:

- **Resurrección de paths liberados por operaciones de contenido.** Las operaciones de contenido
  (`patch_frontmatter`, `replace_body`, `replace_text`, `edit_section`) juzgan existencia y
  contenido contra el `DocumentSet` **inicial** del plan, igual que `create`/`move` antes de H04, y
  por tanto pueden **resucitar** un documento que una operación anterior del mismo plan ya
  borró/movió: `[delete A, patch_frontmatter A]` resucita `A.md` con solo el frontmatter nuevo (sin
  el cuerpo original); `[delete A, replace_body A]` lo resucita con el cuerpo nuevo; `[move A→B,
  replace_text A]` deja **dos** documentos vivos (`B.md` con el contenido movido y `A.md`
  resucitado). Verificado contra el binario en los tres commits.
- **Move-chains por ocupación del origen.** Cualquier cadena donde el `from` de un `move` (o el
  path que referencia una operación de contenido) fue producido por una operación anterior del
  mismo plan comparte la misma causa: `[create X, move X→Y]` (`NORMALIZE_TARGET_NOT_FOUND`),
  `[move A→B, move B→C]`, y el swap `A↔B` vía path temporal. Es la contraparte simétrica del
  acumulado de **ocupación del destino** que H04 sí resuelve para `create`/`move`.

**Severidad estimada**: media. Ninguno de los dos ocurre con una sola operación ni con las
secuencias de un solo paso que el testbench probó por defecto — requieren que un agente construya
deliberadamente un plan multi-operación sobre paths relacionados. El caso más grave es la
resurrección-tras-move (`[move A→B, replace_text A]`), que **duplica** conocimiento en vez de solo
perderlo o bloquearse; los demás son pérdida silenciosa de una operación intermedia o un rechazo
sin salida (`NORMALIZE_TARGET_NOT_FOUND`), no una escritura que destruye sin diagnóstico.

**Destino propuesto**: una historia futura de «normalización con estado de contenido acumulado» —
generalizar el acumulado de H04 (hoy limitado a **ocupación de path**) para que
`document_body`/`op_ref_path` resuelvan también contra el **contenido** que las operaciones previas
del mismo plan producen, no solo contra si el path está libre u ocupado. Es un cambio de forma más
amplio que el de H04 (toca la resolución de cuerpo, no solo el guard de existencia) y queda **fuera
de la campaña actual** (E28); candidato natural para una fase posterior de la campaña de bugfixes
del testbench homelab (`docs/qa/campana-bugfixes-2026-08.md`) o para `decisiones/23-hallazgos-testbench-homelab.md`
si se decide priorizarlo junto con el resto de hallazgos pendientes de esa ficha.
