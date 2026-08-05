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
