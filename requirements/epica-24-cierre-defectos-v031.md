# E24 — Cierre de defectos de la v0.3.0: datos, recuperación, consulta y superficie

> **Fase**: posterior a la publicación de v0.3.0 (tag `ee127ee`). No es una fase de `§20.14`: es la
> épica de **corrección** que salda lo que la revisión de la v0.3.0 destapó **después** de publicar.
> **Objetivo de la épica**: que v0.3.1 salga sin pérdida silenciosa de datos, con un workspace que se
> pueda seguir usando después de un crash, con un lenguaje de consulta que no responda a lo que no
> entiende, y con una suite que muerda donde el sondeo demostró que no muerde.
> Referencias maestras: `ARCHITECTURE.md §20` (entero), `docs/REFACTOR_PHASE_2.md`,
> `contracts/mcp.yml`, `CLAUDE.md` (invariantes).

**Origen**: revisión de la v0.3.0 (2026-07-28). Igual que en E23, **ningún defecto se dedujo leyendo
código**: los cinco se reprodujeron ejecutando el binario `lodestar-mcp` por JSON-RPC sobre stdio y
la CLI `lodestar` contra workspaces de prueba, con un arnés de sesión viva. Cada historia de los
bloques A–C lleva el síntoma reproducible que la motiva.

La v0.3.0 **pasa todas sus puertas** —437 tests, `clippy -D warnings`, los 4 de crash-recovery tras
`--features test-failpoints`, pureza del core— y el invariante nuclear aguanta 30 `SIGKILL` reales
durante `change_apply` sin dejar ni un `.md` a medias. Nada de lo que sigue lo contradice: son
defectos que la suite **no mira**.

**Principio rector**: el invariante #1 de `CLAUDE.md` — *«los `.md` en disco son la única fuente de
verdad»* — y el propósito declarado de `knowledge_check` en `§20.9`: *«¿puede Lodestar interpretar y
modificar esto de forma segura?»*. Un documento cuyo frontmatter se pierde en silencio mientras la
validación responde `VÁLIDO` incumple los dos a la vez.

**Fuera de alcance (explícito)**:

- **Rechazar parámetros NO declarados** en la superficie MCP. Contradice la regla de la casa escrita
  en `contracts/mcp.yml:276-290` («el servidor valida los VALORES de los parámetros que declara, e
  IGNORA lo que no declara»), en la cabecera de `descubribilidad.rs:59-67` y en `tools.rs:57-60`. No
  es una extensión de la política: es revisarla. **E24-H18** la registra como decisión abierta en
  `decisiones/`; no se implementa aquí.
- **`lodestar recover` como subcomando.** La recuperación se resuelve de forma transparente
  (E24-H03); el hueco residual para una persona o un CI en perfil `readonly` queda declarado en
  E24-H04, no cerrado.
- **Conectar el store** (`decisiones §14`, abierta desde E23-H16). Sigue sin consumidor.

---

## Bloque A — Pérdida silenciosa de datos

### E24-H01 — Un BOM deja de tragarse el frontmatter

- **Objetivo**: un `.md` que empieza por BOM UTF-8 se interpreta y se reescribe **sin perder
  metadata**, y su BOM se conserva byte a byte.
- **Síntoma reproducible**: fichero
  `b'\xef\xbb\xbf---\nstatus: draft\nowner: ana\n---\n\n# Con BOM\n\ncuerpo original\n'`.
  - `knowledge_get` devuelve `frontmatter: {}` y un `body` que contiene el bloque `---…---` entero.
  - `document.has_frontmatter = false`.
  - `knowledge_check` y `lodestar check` responden **VÁLIDO, 0 errores, 0 avisos, 0 diagnósticos**.
  - Y al escribir encima, `change_plan`+`change_apply` con
    `{"op":"patch_frontmatter","path":"bom.md","patch":{"status":"review"}}` produce:
    ```
    ---
    status: review
    ---

    ﻿---
    status: draft
    owner: ana
    ---

    # Con BOM
    ```
    Dos bloques; el original degradado a texto del cuerpo. Un `replace_body` posterior **destruye
    `owner: ana` y `status: draft` para siempre**.
- **Referencias**: `ARCHITECTURE.md §20.2` (modelo documental) · `crates/lodestar-core/src/model.rs`
  (`split_front:56`, `build_raw:297`, `patch_frontmatter:356`) · `CLAUDE.md` invariante #1. Es la
  misma familia que la corrupción que cerró `E23-H03` (`---\n{}\n---` sobre documento sin
  frontmatter), con otro disparador.
- **Alcance**:
  - `model::split_front` reconoce un BOM inicial al detectar el bloque. Hoy hace
    `if !raw.starts_with("---") { return SplitFront::Sin }`; con BOM, `raw` empieza por `\u{feff}---`
    y cae en `Sin`. Los offsets (`span`, `body_start`) siguen siendo **posiciones de byte válidas
    sobre `raw`**: la invariante `raw[span] == fm.raw` no se rompe.
  - `model::patch_frontmatter`, rama `SplitFront::Sin` (líneas 367-382), deja de anteponer un bloque
    por delante del BOM.
  - `model::build_raw` reemite el BOM si el documento lo tenía.
  - **El BOM NO se normaliza en la lectura de disco** (`crates/lodestar-workspace/src/discovery.rs:283`).
    `types::workspace_revision` hashea los bytes crudos del `FileMap`, así que strippear al leer y no
    al escribir declararía un cambio espurio en cada round-trip.
  - Un solo punto de arreglo cubre toda la ingesta: los 7 sitios que construyen documentos pasan por
    `model::parse_file` (`document_set.rs:53`, `plan.rs:349`/`:994`, `app/lib.rs:569`/`:725`/`:2278`,
    `cli/commands.rs:206`).
- **Criterios de aceptación**:
  - **Dado** un `.md` con BOM y frontmatter válido, **Cuando** se lee con `knowledge_get`,
    **Entonces** `frontmatter` trae las claves reales y `document.has_frontmatter` es `true` →
    `bom_no_se_traga_el_frontmatter`.
  - **Dado** ese documento, **Cuando** se le aplica `patch_frontmatter`, **Entonces** el resultado
    tiene **un solo** bloque de frontmatter, conserva las claves que no se tocaron y **empieza por el
    BOM** → `patch_sobre_bom_no_duplica_bloque`.
  - **Dado** ese documento, **Cuando** se lee y se reescribe sin cambios, **Entonces** los bytes son
    idénticos y la `WorkspaceRevision` no cambia → `bom_roundtrip_byte_a_byte` (control anti-vacuo:
    sin esto, un arreglo que strippee el BOM también pasaría los dos anteriores).
  - **Dado** un `.md` sin BOM, **Cuando** se parsea, **Entonces** el comportamiento es idéntico al
    de v0.3.0 → los tests existentes de `documento.rs` siguen verdes sin tocarse.
- **Dependencias**: ninguna.
- **Pruebas**: `crates/lodestar-core/tests/documento.rs` + `crates/lodestar-mcp/tests/mcp.rs`.
- **Frontera (mcp.yml)**: no (cambia el parseo, no la forma del wire).
- **Consecuencia declarada (hallada por el juez ciego, medida)**: al pasar a **interpretar** el
  documento, `lodestar check` puede cambiar de exit **0** a exit **1** en workspaces que hoy pasan:

  | | v0.3.0 | con el arreglo |
  |---|---|---|
  | BOM + bloque sin cerrar | `[]` | `[FM-UNCLOSED, Err]` |
  | BOM + YAML inválido | `[]` | `[FM-YAML-INVALID, Err]` |

  Es el comportamiento **correcto** y el propio principio rector de la épica (dejar de responder
  VÁLIDO sobre lo que no se entiende), pero es un cambio de veredicto de la puerta de CI para bases
  existentes. **Debe ir en la nota de release de v0.3.1** (E24-H18).

### E24-H02 — Un BOM es visible, no silencioso

- **Objetivo**: el usuario se entera de que tiene un BOM, porque no es portable.
- **Síntoma reproducible**: el mismo fichero de H01 produce `knowledge_check → valid=True, 0
  diagnósticos` y `lodestar check → exit 0 · VÁLIDO`, aun cuando su metadata era invisible para el
  motor.
- **Referencias**: `ARCHITECTURE.md §20.9` (catálogo de diagnósticos) ·
  `crates/lodestar-core/src/conform.rs` (`validate_file:30`) · `crates/lodestar-core/src/types.rs`
  (`CheckCode:178-253`, `as_str:255`). Precedente de forma: `LINK-CASE-MISMATCH`, que también avisa
  de un problema de portabilidad entre sistemas de ficheros sin bloquear.
- **Alcance**:
  - Nueva variante de `CheckCode` con severidad **aviso** (`Warn`), emitida desde
    `conform::validate_file`. El catálogo de `CheckCode` **no está congelado** —lo congelado es
    `ErrorCode`, 16 filas— y ningún test fija su tamaño; `CheckCode::as_str()` es un `match`
    exhaustivo, así que añadir una variante sin actualizarlo **no compila**.
  - Actualizar la tabla de diagnósticos de `ARCHITECTURE.md §20.9` y el doc del fixture
    `crates/lodestar-fixtures/src/lib.rs:188-205`, que enumera los códigos.
- **Criterios de aceptación**:
  - **Dado** un `.md` con BOM, **Cuando** se llama a `knowledge_check(scope: workspace)`,
    **Entonces** aparece el diagnóstico con `level: warn` y el documento sigue siendo válido
    (`valid: true`, exit 0) → `bom_emite_aviso_sin_bloquear`.
  - **Dado** un `.md` sin BOM, **Cuando** se valida, **Entonces** no aparece ese código →
    `sin_bom_no_hay_aviso` (control anti-vacuo).
- **Dependencias**: **E24-H01** (el diagnóstico describe un documento que ya se parsea bien).
- **Pruebas**: `crates/lodestar-core/tests/diagnosticos.rs` + `crates/lodestar-cli/tests/cli.rs`.
- **Frontera (mcp.yml)**: sí — el catálogo de `CheckCode` viaja en `knowledge_check`.

---

## Bloque B — Recuperación tras un crash

### E24-H03 — `change_plan` planifica sobre el estado ya recuperado

- **Objetivo**: tras un crash, la primera pareja `change_plan` + `change_apply` de un agente
  **funciona**, sin reintento.
- **Síntoma reproducible**: `SIGKILL` al servidor a mitad de `change_apply` (move de 121 ficheros con
  `rewriteInboundLinks`). Al reabrir: `workspace_status.recovery.pendingTransaction: true`. El
  siguiente `change_plan` + `change_apply` falla con **`WRITE_CONFLICT`**; el segundo intento
  funciona. Reproducido en 10 de las 11 veces en que el crash dejó transacción pendiente.
  El código además **miente**: `WRITE_CONFLICT` significa «otro escritor lo modificó entre el plan y
  el apply», y aquí quien lo modificó fue la recuperación del propio Lodestar.
- **Referencias**: `ARCHITECTURE.md §19.4` (modelo transaccional) ·
  `crates/lodestar-app/src/lib.rs` (`change_plan:1399`) ·
  `crates/lodestar-workspace/src/transaction.rs` (`apply_transaction:100`, pasos (2) y (7)) ·
  `crates/lodestar-workspace/src/lib.rs` (`reverify_base_revision:178`).
- **Cadena exacta del defecto**:
  1. `change_plan` lee `document_set()` sobre el disco **sin recuperar** (renames parciales visibles)
     y fija ahí `base_revision`; persiste el plan con esa base.
  2. `change_apply` recomputa el `planHash` sobre el mismo estado sin recuperar → coincide, **no**
     salta `PLAN_STALE`.
  3. `apply_transaction` paso (2) llama a `recover()`, que restaura los originales.
  4. Paso (7) `reverify_base_revision` compara la base pre-recuperación contra la revisión
     post-recuperación → `WriteConflict`.
- **Alcance**:
  - `App::change_plan` recupera si hay recuperación pendiente **antes** de leer el `DocumentSet`, y
    lo hace **bajo el lock de publicación** (mismo lock que toma `apply_transaction`), para que dos
    planificadores no recuperen a la vez.
  - **Matiz de criterio a acotar**: en ese caso el plan **escribe**. El criterio de `plan_no_escribe`
    pasa de «no toca disco jamás» a «no publica el resultado del plan»: reparar una transacción
    interrumpida no es publicar. El test existente se amplía, no se borra.
  - Sin efecto en perfil `readonly`: las tools de cambio no están disponibles ahí.
- **Criterios de aceptación**:
  - **Dado** un workspace con una transacción interrumpida (journal no-`done` + renames parciales),
    **Cuando** un agente hace `change_plan` y luego `change_apply`, **Entonces** el apply tiene éxito
    **al primer intento** → `apply_tras_crash_no_da_write_conflict`.
  - **Dado** ese mismo workspace, **Cuando** se llama a `change_plan`, **Entonces** el
    `baseWorkspaceRevision` del plan es el del estado **recuperado**, no el del estado parcial →
    `plan_tras_crash_parte_del_estado_recuperado`.
  - **Dado** un workspace **sin** recuperación pendiente, **Cuando** se llama a `change_plan`,
    **Entonces** no se escribe absolutamente nada en el canónico → `plan_no_escribe` (ampliado;
    control anti-vacuo: el arreglo no puede convertir el plan en un escritor habitual).
- **Dependencias**: ninguna.
- **Pruebas**: `crates/lodestar-app/tests/plan.rs` + `crates/lodestar-mcp/tests/e2e_ciclo_vida.rs`.
- **Frontera (mcp.yml)**: no (cambia la semántica de `change_plan`, no sus parámetros).

### E24-H04 — Un workspace recuperable deja de presentarse como roto

- **Objetivo**: quien **lee** un workspace con recuperación pendiente sabe que lo que ve es
  transitorio y recuperable, no daño real.
- **Síntoma reproducible**: sobre el workspace a medias de H03, `lodestar check` responde
  `exit=1 · "121 documentos · 120 con errores · 0 avisos · NO VÁLIDO"` y `knowledge_check` reporta
  120 `LINK-TARGET-MISSING`, sin una sola mención de que hay una transacción pendiente. Un CI lo
  leería como una KB rota.
- **Referencias**: `crates/lodestar-cli/src/commands.rs` (`check:26`) ·
  `crates/lodestar-app/src/lib.rs` (`full_analysis:1029`, `workspace_status:426`) ·
  `crates/lodestar-workspace/src/lib.rs` (`guard_recovery:356`) ·
  `crates/lodestar-core/src/types.rs` (`ErrorCode::WorkspaceRecoveryRequired`).
- **Hallazgo colateral**: `WORKSPACE_RECOVERY_REQUIRED` es hoy **inalcanzable por el wire**. El único
  gate que lo emitiría, `guard_recovery`, no tiene **ningún** llamador en `lodestar-app`,
  `lodestar-mcp` ni `lodestar-cli`; el otro (`publish.rs:93`) corre después de la recuperación
  automática del paso (2), así que su lista siempre está vacía. Es uno de los códigos del catálogo
  sin emisor real (`contracts/mcp.yml:616`).
- **Alcance**:
  - `lodestar check` y `knowledge_check` informan de la recuperación pendiente. **Sin escribir
    nada**: la apertura sigue siendo hermética (`E23-H12`), así que informan, no reparan.
  - Decidir y fijar el veredicto: un workspace con recuperación pendiente no debe reportarse como
    «NO VÁLIDO» por diagnósticos que son artefactos de la transacción a medias.
  - `WORKSPACE_RECOVERY_REQUIRED` pasa a tener un emisor real alcanzable desde el wire, y sale de la
    lista `codigos_sin_emisor` de `contracts/mcp.yml`.
- **Riesgo residual declarado (no se cierra en esta épica)**: con la recuperación transparente de
  H03, el único disparador de la reparación sigue siendo `change_plan`. Una persona, un CI o una
  sesión en perfil `readonly` **ven** el aviso pero no pueden repararlo. Un `lodestar recover`
  cerraría el hueco; queda fuera de alcance por decisión explícita.
- **Criterios de aceptación**:
  - **Dado** un workspace con transacción interrumpida, **Cuando** se corre `lodestar check`,
    **Entonces** la salida nombra la recuperación pendiente → `check_avisa_de_recuperacion_pendiente`.
  - **Dado** ese workspace, **Cuando** se llama a `knowledge_check`, **Entonces** el informe lo
    refleja y `lodestar check` coincide con él (invariante #3, «una sola verdad computada») →
    `check_y_knowledge_check_coinciden_con_recuperacion_pendiente`.
  - **Dado** un workspace **sin** recuperación pendiente, **Cuando** se corre lo mismo, **Entonces**
    no aparece ningún aviso de recuperación → control anti-vacuo.
  - **Dado** un workspace con recuperación pendiente, **Cuando** una escritura no puede proceder,
    **Entonces** el código de wire es `WORKSPACE_RECOVERY_REQUIRED` y no otro →
    `recovery_required_llega_al_wire`.
- **Dependencias**: **E24-H03**.
- **Pruebas**: `crates/lodestar-cli/tests/cli.rs` + `crates/lodestar-mcp/tests/mcp.rs`.
- **Frontera (mcp.yml)**: sí — `codigos_sin_emisor` pierde una fila.

### E24-H05 — Una transacción fallida no deja staging

- **Objetivo**: ninguna transacción que falle deja basura en `.lodestar/runtime/staging/`.
- **Síntoma reproducible**: tras el `WRITE_CONFLICT` de H03, `.lodestar/runtime/staging/<txn>/`
  contiene el árbol `.md` **completo** del resultado (121 ficheros en la reproducción) y nadie lo
  borra nunca. Crecimiento sin cota: el flujo que produce la basura es exactamente el que no la
  recoge.
- **Referencias**: `crates/lodestar-workspace/src/staging.rs` (`StagingDir:31`,
  `validate_staging:181`) · `crates/lodestar-workspace/src/transaction.rs` (paso (11), línea 164) ·
  precedente RAII: `crates/lodestar-workspace/src/lock.rs:35` (`impl Drop for WorkspaceLock`).
- **Alcance**:
  - `StagingDir` **no tiene `Drop`** (el único `impl Drop` del crate es el del lock). Los pasos (7) a
    (10) de `apply_transaction` salen por `?` saltándose el `remove_dir_all` del paso (11).
  - Hacerlo RAII: el `Drop` limpia salvo que la transacción lo haya «consumido» explícitamente en el
    camino feliz. El rustdoc actual del tipo (líneas 28-30) declara lo contrario y hay que
    actualizarlo.
- **Criterios de aceptación**:
  - **Dado** un plan cuya base ya no es la revisión actual, **Cuando** `change_apply` falla con
    `WRITE_CONFLICT`, **Entonces** `.lodestar/runtime/staging/` queda vacío →
    `staging_no_sobrevive_a_una_transaccion_fallida`.
  - **Dado** un apply correcto, **Cuando** termina, **Entonces** el staging también se limpia y el
    resultado publicado es el esperado → control anti-vacuo (el `Drop` no puede borrar lo que la
    publicación aún necesita).
- **Dependencias**: ninguna.
- **Pruebas**: `crates/lodestar-workspace/tests/transactions.rs`.
- **Frontera (mcp.yml)**: no.

### E24-H06 — El GC recoge huérfanos, y corre también cuando algo falla

- **Objetivo**: el plano de control (`.lodestar/runtime/`) no crece sin límite.
- **Síntoma reproducible**: tres fugas verificadas — (a) el `staging/<txn>/` de H05; (b)
  `recovery/<txn>/.absent` huérfano, porque `backup_originals` escribe el manifiesto **siempre**,
  incluso vacío, y el paso (11) conserva el árbol a propósito; (c)
  `journal/<txn>.json.<pid>-N.lodestar-tmp`, dejado por un `SIGKILL` entre el `File::create` y el
  `rename` de la escritura atómica.
- **Referencias**: `crates/lodestar-workspace/src/receipts.rs` (`gc_receipts:224`) ·
  `crates/lodestar-workspace/src/recovery.rs` (`backup_originals:113`, manifiesto en `:147`) ·
  `crates/lodestar-workspace/src/io.rs` (`tmp_sibling:54`), `journal.rs:228`, `receipts.rs:81` ·
  `crates/lodestar-app/src/lib.rs` (disparos del GC en `:1642` y `:1771`).
- **Alcance**:
  - `gc_receipts` itera hoy **solo** `receipts/`, así que un `staging/<txn>/` cuya transacción nunca
    produjo recibo le es invisible. Invertir el barrido: recorrer `staging/` y `recovery/` y purgar
    lo que no tenga ni journal vivo ni recibo vigente. La convención de nombre lo permite — el mismo
    `txnId` saneado nombra `staging/<id>/`, `recovery/<id>/`, `journal/<id>.json` y
    `receipts/<id>.json` (documentado en `receipts.rs:9-15`).
  - Barrer también los `*.lodestar-tmp` huérfanos.
  - El GC se dispara hoy **solo en el camino de éxito**. Añadir un disparo que cubra el fallo.
  - `backup_originals` deja de escribir el manifiesto `.absent` cuando está vacío.
  - Unificar las **tres copias** copy-paste del patrón `tmp_sibling` (`io.rs`, `journal.rs`,
    `receipts.rs`), cada una con su propio `static SEQ`.
- **Criterios de aceptación**:
  - **Dado** un `runtime/` con un `staging/<txn>/` y un `recovery/<txn>/` sin journal ni recibo,
    **Cuando** corre el GC, **Entonces** ambos desaparecen → `gc_purga_huerfanos_sin_recibo`.
  - **Dado** un `journal/x.json.123-0.lodestar-tmp` huérfano, **Cuando** corre el GC, **Entonces**
    desaparece → `gc_purga_temporales_huerfanos`.
  - **Dado** un `runtime/` con una transacción **en curso** (journal `prepared` + su staging),
    **Cuando** corre el GC, **Entonces** **no** se toca nada de esa transacción →
    `gc_no_toca_transacciones_vivas` (control anti-vacuo: el barrido no puede sabotear una
    publicación en marcha).
  - **Dado** una transacción que solo crea ficheros, **Cuando** se publica, **Entonces** no queda un
    `recovery/<txn>/` con únicamente `.absent` → `sin_manifiesto_absent_vacio`.
- **Dependencias**: **E24-H05**.
- **Pruebas**: `crates/lodestar-workspace/tests/transactions.rs`.
- **Frontera (mcp.yml)**: no.

---

## Bloque C — Lenguaje de consulta · **v0.4.0 · CERRADO**

> **Corte de release (2026-07-29).** H07 y H08 **no entran en v0.3.1**. Las dos cambian resultados
> observables de consultas que hoy se aceptan (`graph.foo = 1` pasa de devolver `[]` a fallar), y una
> revisa un criterio ratificado en E19-H04. Se implementan **después** de publicar v0.3.1, como
> núcleo de v0.4.0.
>
> **Cerrado el 2026-07-29, después de publicar v0.3.1**, como estaba previsto.
>
> **Verificado que no bloquean a nadie de v0.3.1**: ninguna historia del resto de la épica depende de
> H07 ni de H08. La única que las nombraba era H18 (aviso de release), que pierde esa mitad del
> aviso. H07/H08 sí comparten fichero con nadie más de la épica: tocan `core/src/parse.rs`,
> `core/src/eval.rs`, `core/src/filter.rs` y `core/src/types.rs`, que ninguna otra historia de
> v0.3.1 modifica.

### E24-H07 — Los namespaces reservados rechazan lo que no conocen

- **Objetivo**: una consulta con una propiedad inexistente bajo `graph.`/`document.` **falla**, en
  vez de devolver una respuesta vacía indistinguible de un resultado legítimo.
- **Síntoma reproducible**:
  ```
  graph.backlinks = 0     -> ['a.md','solo1.md','solo2.md']   correcto
  graph.backlink  = 0     -> []      typo: respuesta, no error
  graph.foo = 1           -> []
  document.path  = "a.md" -> ['a.md']                          correcto
  document.pathh = "a.md" -> []      typo: respuesta, no error
  ```
  Un agente que se equivoca de propiedad recibe «no hay resultados». Es peor que un error: es una
  respuesta silenciosamente equivocada.
- **Referencias**: `ARCHITECTURE.md §20.8` (*«las calculadas EXIGEN namespace»*) ·
  `crates/lodestar-core/src/eval.rs` (`resolver_campo:132`, `resolver_document:149`,
  `resolver_graph:169`, `eval_comparison:98`) · `crates/lodestar-core/src/parse.rs`
  (`build_field_path:443`) · `crates/lodestar-core/src/filter.rs` (`normalizar_campo:136`).
- **Cambio de criterio a ratificar**: el comentario de `crates/lodestar-core/tests/consulta.rs:916-921`
  y el rustdoc de `eval.rs:130` consagran hoy lo contrario — *«`None` = propiedad ausente (una
  sub-clave de namespace desconocida, `graph.foo`, también)»*. Fue una decisión deliberada de
  **E19-H04**. Esta historia la revisa: bajo un namespace **reservado**, una propiedad desconocida es
  un error de consulta, no una ausencia.
- **Alcance**:
  - **Fuente única de verdad para las propiedades válidas.** Hoy la lista vive **solo** en los brazos
    `match` de `eval.rs:153` (`path`, `title`, `has_frontmatter`) y `eval.rs:178` (`backlinks`,
    `outgoing_links`, `dangling_links`, `isolated`). Nada en `types.rs`, nada en el contrato.
    Extraerla a `core::types` (dos `const &[&str]`) para que el validador y el evaluador no puedan
    divergir sin que nada lo detecte.
  - **Rechazo en `parse::build_field_path`** — el **único** punto compartido por `where`, `filter` y
    `has`/`missing` (sus tres llamantes son `parse.rs:284`, `parse.rs:316` y `filter.rs:137`). Así la
    equivalencia `where ≡ filter` se preserva **por construcción**, y el error se produce al compilar
    la consulta, no por documento.
  - **No** tocar `FieldPath::from_segments`: `ParsedFrontmatter::walk` (`types.rs:604`) lo usa con
    claves YAML arbitrarias del usuario, y validar ahí reventaría el catálogo de metadata de
    cualquier documento con una clave `graph`.
- **Criterios de aceptación**:
  - **Dado** `where: "graph.backlink = 0"`, **Cuando** se llama a `knowledge_search`, **Entonces**
    falla con `INVALID_SCHEMA` y el mensaje nombra las propiedades válidas →
    `namespace_reservado_rechaza_propiedad_desconocida`.
  - **Dado** el `filter` JSON equivalente, **Cuando** se llama, **Entonces** falla igual y con el
    mismo código → `filtro_con_namespace_desconocido_es_invalid_schema`.
  - **Dado** `where: "status_inventado = x"` (campo de frontmatter inexistente, **sin** namespace
    reservado), **Cuando** se llama, **Entonces** devuelve `[]` sin error →
    `campo_de_frontmatter_inexistente_sigue_siendo_ausencia` (control anti-vacuo: el rechazo es solo
    bajo namespace reservado).
  - **Dado** las 7 propiedades válidas, **Cuando** se consultan, **Entonces** siguen funcionando
    exactamente igual → los tests de `consulta.rs:909-1100` siguen verdes.
- **Dependencias**: ninguna. **Requiere ratificación explícita** (revisa un criterio de E19-H04).
- **Pruebas**: `crates/lodestar-core/tests/consulta.rs` + `crates/lodestar-mcp/tests/mcp.rs`.
- **Frontera (mcp.yml)**: sí — cambia el conjunto de errores de `knowledge_search` y `change_plan`.

### E24-H08 — `frontmatter.` vuelve a ser un anclaje real

- **Objetivo**: el frontmatter propio del usuario es **siempre** alcanzable, incluso si sus claves se
  llaman `graph` o `document`.
- **Síntoma reproducible**: con `b.md` conteniendo `graph:\n  backlinks: 7` en su frontmatter:
  ```
  metadata_inspect(catalog) -> lista `graph.backlinks` (presentIn=1, tipo number)   ← lo anuncia
  graph.backlinks = 7             -> []
  frontmatter.graph.backlinks = 7 -> []                                             ← no hay escape
  ```
  `metadata_inspect` anuncia campos que **ninguna consulta puede alcanzar**. Rompe justo el flujo que
  las `instructions` del servidor recomiendan (paso 4 `metadata_inspect` → paso 2
  `knowledge_search`), y contradice el propósito de `E23-H11`: la descubribilidad.
- **Referencias**: `crates/lodestar-core/src/parse.rs` (`build_field_path:443`) ·
  `crates/lodestar-core/src/types.rs` (`FieldPath:394`) · `crates/lodestar-core/src/eval.rs`
  (`resolver_campo:132`, `propiedad_presente:216`).
- **Causa raíz**: `build_field_path` **descarta** el prefijo `frontmatter` («abreviatura»), así que
  `frontmatter.graph.backlinks` se convierte en `["graph","backlinks"]` y `resolver_campo` lo captura
  por el namespace. El anclaje explícito se pierde por completo en la normalización, y `FieldPath` no
  tiene forma de representarlo (`pub struct FieldPath(Vec<String>)`, sin flag).
- **Alcance**:
  - `FieldPath` gana la noción de **anclaje explícito al frontmatter**, y `build_field_path` deja de
    perderla. `resolver_campo` respeta el anclaje y va directo al frontmatter sin mirar namespaces.
  - **Sub-bug incluido**: `propiedad_presente` (`eval.rs:216`) **ignora los namespaces por completo**
    — hace `FieldPath::parse` y va directo a `fm.get(&path)`, así que `has(graph.backlinks)` consulta
    la clave de frontmatter `graph.backlinks`, no el grafo. Hoy es la **única** vía por la que un
    frontmatter `graph:` es alcanzable, por accidente.
  - Choca de frente con `consulta.rs:838` (`abreviatura_de_namespace`), que fija que
    `frontmatter.status` produce un `FieldPath` **desnudo**. Se **amplía**, no se borra: la
    abreviatura sigue funcionando; lo que cambia es que el prefijo explícito deja de ser un no-op.
- **Criterios de aceptación**:
  - **Dado** un documento con frontmatter `graph: {backlinks: 7}`, **Cuando** se consulta
    `frontmatter.graph.backlinks = 7`, **Entonces** lo devuelve →
    `anclaje_frontmatter_alcanza_clave_reservada`.
  - **Dado** ese mismo documento, **Cuando** se consulta `graph.backlinks = 0`, **Entonces** responde
    el **grafo**, no el frontmatter → `namespace_sigue_ganando_sin_anclaje` (control anti-vacuo).
  - **Dado** `has(graph.backlinks)` y `has(frontmatter.graph)`, **Cuando** se evalúan, **Entonces**
    cada uno consulta su ámbito → `has_respeta_los_namespaces`.
  - **Dado** `frontmatter.status = draft`, **Cuando** se evalúa, **Entonces** sigue funcionando igual
    que en v0.3.0 → `abreviatura_de_namespace` (ampliado).
- **Dependencias**: **E24-H07** (comparten `build_field_path` y la fuente única de propiedades).
- **Pruebas**: `crates/lodestar-core/tests/consulta.rs` + `crates/lodestar-mcp/tests/descubribilidad.rs`.
- **Frontera (mcp.yml)**: no.

---

## Bloque D — Superficie honesta

### E24-H09 — Se validan los valores de los parámetros declarados

- **Objetivo**: cumplir la política que el contrato ya declara, en vez de aceptar valores imposibles
  y caer al default en silencio.
- **Síntoma reproducible**: sobre un workspace con 2 documentos —
  ```
  limit: 0      -> 0 resultados   (el schema declara "minimum": 1) ← parece «no hay nada»
  limit: "10"   -> 2 resultados   (ignorado, cae al default 20)
  limit: -5     -> 2 resultados
  limit: 9999   -> 2 resultados   (el schema declara "maximum": 100)
  text: 42      -> 2 resultados
  includeSuggestedFixes: "true" -> tratado como false
  graph_query depth: "3"        -> cae al default 1
  ```
- **Referencias**: `contracts/mcp.yml:276-290` — *«el servidor **valida los VALORES de los parámetros
  que declara**»* · `crates/lodestar-mcp/src/tools.rs` (`call:261-510`).
- **Precisión importante**: esto **no** es un cambio de política, es la política vigente sin
  implementar. Rechazar parámetros **no declarados** sí sería un cambio, y queda fuera de alcance
  (ver la cabecera de la épica y E24-H18).
- **Alcance**:
  - Los 10 brazos de `tools::call` leen ~30 parámetros a mano
    (`params.get("x").and_then(Value::as_u64).unwrap_or(…)`), cada uno con su propia política de
    default. **No existe ningún helper de extracción** en `tools.rs` (el único helper es
    `to_json:512`). Introducir uno que valide tipo y rango declarados y produzca `INVALID_SCHEMA`.
  - Alcanza a: `limit` de `knowledge_search` y `knowledge_check`, `depth` de `graph_query` e
    `impact_analyze`, `includeSuggestedFixes`, `text`, `cursor`.
- **Criterios de aceptación**:
  - **Dado** `knowledge_search` con `limit: 0`, `limit: 9999`, `limit: -5` o `limit: "10"`,
    **Cuando** se llama, **Entonces** cada uno falla con `INVALID_SCHEMA` →
    `limit_fuera_de_rango_es_invalid_schema`.
  - **Dado** `includeSuggestedFixes: "true"` o `depth: "3"`, **Cuando** se llama, **Entonces** fallan
    con `INVALID_SCHEMA` en vez de caer al default → `tipo_incorrecto_es_invalid_schema`.
  - **Dado** los mismos parámetros con valores válidos, **Cuando** se llaman, **Entonces** se
    comportan exactamente igual que en v0.3.0 → control anti-vacuo.
- **Dependencias**: ninguna.
- **Pruebas**: `crates/lodestar-mcp/tests/mcp.rs`.
- **Frontera (mcp.yml)**: sí — `meta.validacion_de_argumentos` deja de tener esta mitad como deuda.

### E24-H10 — Todo error de superficie lleva código del catálogo

- **Objetivo**: un agente puede ramificar por código en **cualquier** error, sin parsear prosa.
- **Síntoma reproducible**: 10 de 21 errores de superficie viajan como texto suelto —
  ```
  knowledge_get sin ref      -> 'falta el parámetro «ref»'
  metadata_inspect sin mode  -> 'falta el parámetro «mode»'
  knowledge_check scope.kind inválido -> 'unknown variant `nope`, expected one of …'
  where: "status ="          -> 'error del núcleo: «where» inválido: se esperaba un valor…'
  filter inválido            -> '…filtro JSON malformado: data did not match any variant of
                                untagged enum W'      ← interno de serde filtrado al agente
  ```
- **Referencias**: `crates/lodestar-core/src/types.rs` (`ErrorCode:1216-1261`) ·
  `crates/lodestar-app/src/lib.rs` (`build_search_expression:2884`, `workspace_error_code:160`,
  `build_selection_expression:2302`) · `crates/lodestar-mcp/src/tools.rs`.
- **Asimetría concreta a cerrar**: la **misma** consulta malformada produce **dos códigos distintos
  según la tool**. Por `knowledge_search`, `build_search_expression` la envuelve en
  `WorkspaceError::Core` y `workspace_error_code` la mapea a **`INTERNAL_IO_ERROR`**; por
  `change_plan.selection`, `build_selection_expression` ya devuelve **`INVALID_SCHEMA`**.
- **Alcance**:
  - `where`/`filter` malformado → `INVALID_SCHEMA` en **ambos** caminos.
  - Parámetro obligatorio ausente o con variante inválida → `INVALID_SCHEMA`.
  - Dejar de filtrar internos de `serde` en los mensajes de wire.
  - **No** se añade ningún `ErrorCode`: el catálogo sigue teniendo 16 filas.
- **Criterios de aceptación**:
  - **Dado** `where: "status ="` (sintaxis rota), **Cuando** se llama a `knowledge_search`,
    **Entonces** el error abre con `INVALID_SCHEMA` → `where_malformado_es_invalid_schema`.
  - **Dado** esa misma consulta como `selection` de `change_plan`, **Cuando** se llama, **Entonces**
    el código es el mismo → `misma_consulta_mismo_codigo_en_las_dos_tools`.
  - **Dado** cada tool sin su parámetro obligatorio, **Cuando** se llama, **Entonces** todas fallan
    con `INVALID_SCHEMA` → `parametro_obligatorio_ausente_es_invalid_schema`.
  - **Dado** un `filter` JSON malformado, **Cuando** se llama, **Entonces** el mensaje no contiene
    `"untagged enum"` → `errores_no_filtran_internos_de_serde`.
  - **Dado** el catálogo, **Cuando** se cuenta, **Entonces** sigue teniendo 16 filas → control
    anti-vacuo (el arreglo no puede consistir en inventar códigos).
- **Dependencias**: **E24-H09** (comparten el helper de extracción).
- **Pruebas**: `crates/lodestar-mcp/tests/mcp.rs` + `crates/lodestar-app/tests/error.rs`.
- **Frontera (mcp.yml)**: sí — cambia la lista `errores` de varias tools.

### E24-H11 — `knowledge_get` devuelve `title`

- **Objetivo**: leer un documento da su título, sin tener que volver a buscarlo.
- **Síntoma reproducible**: `knowledge_get` devuelve un `DocumentView` con claves
  `['body','frontmatter','path','revision']` — sin `title`. En cambio `knowledge_search` y
  `graph_query` **sí** lo traen, correctamente derivado (`frontmatter.title` → primer H1 → nombre de
  fichero). El `include` es un enum cerrado, así que tampoco se puede pedir.
- **Referencias**: `ARCHITECTURE.md §20.2` (título derivado) · `crates/lodestar-app/src/lib.rs`
  (`DocumentView`, `knowledge_get:725`) · `crates/lodestar-mcp/src/tools.rs` (brazo
  `knowledge_get:307`).
- **Alcance**: `DocumentView` gana `title`, derivado por la misma lógica que ya usan `SearchResult` y
  `GraphNode` — **sin duplicarla** (invariante #3, una sola verdad computada).
- **Criterios de aceptación**:
  - **Dado** un documento con `title` en frontmatter, otro con solo un H1 y otro sin nada, **Cuando**
    se leen con `knowledge_get`, **Entonces** cada uno trae el título derivado por la cascada de
    `§20.2` → `get_devuelve_titulo_derivado`.
  - **Dado** los mismos documentos, **Cuando** se comparan `knowledge_get.title` y
    `knowledge_search.title`, **Entonces** coinciden → `titulo_coincide_entre_get_y_search` (control
    anti-vacuo: no puede ser una segunda implementación).
- **Dependencias**: ninguna.
- **Pruebas**: `crates/lodestar-mcp/tests/mcp.rs`.
- **Frontera (mcp.yml)**: sí — cambia el `retorno` de `knowledge_get`.

---

## Bloque E — Descubrimiento

### E24-H12 — Los huecos de descubrimiento dejan de ser silenciosos

- **Objetivo**: lo que el motor no descubre, lo dice.
- **Síntoma reproducible**: sobre un árbol de prueba —
  - `README.MD` (extensión en mayúsculas) **no se descubre**, y no se emite ningún diagnóstico.
  - Un symlink **de directorio** no se sigue y **no** emite `SYMLINK-UNSUPPORTED`.
  - Un enlace `[x](guias\sub\hoja.md)` en Unix se clasifica como `workspaceDirectory`.
- **Referencias**: `IMPLEMENTATION_STATUS.md:704-708` (los tres están declarados como «no
  regresiones» desde el juez de E15-H07, pero vivos) · `crates/lodestar-workspace/src/discovery.rs`
  · `crates/lodestar-core/src/links.rs`.
- **Alcance**: fijar el comportamiento **con test**, sea cual sea el elegido en cada caso: descubrir,
  o no descubrir y diagnosticar. Lo que no puede seguir es el silencio.
- **Criterios de aceptación**:
  - **Dado** un `README.MD`, **Cuando** se descubre el workspace, **Entonces** el comportamiento está
    fijado por test y, si no se descubre, hay diagnóstico → `extension_en_mayusculas_fijada`.
  - **Dado** un symlink de directorio, **Cuando** se descubre, **Entonces** se emite
    `SYMLINK-UNSUPPORTED` → `symlink_de_directorio_diagnostica`.
  - **Dado** un enlace con barras invertidas en Unix, **Cuando** se clasifica, **Entonces** el
    resultado está fijado por test → `barra_invertida_en_unix_fijada`.
- **Dependencias**: ninguna.
- **Pruebas**: `crates/lodestar-workspace/tests/discovery.rs` + `crates/lodestar-core/tests/enlaces.rs`.
- **Frontera (mcp.yml)**: no.

---

## Bloque F — Endurecimiento de la suite

### E24-H13 — Seam real de failpoints

- **Objetivo**: que la garantía nuclear se verifique contra el **orquestador real**, no contra una
  reconstrucción.
- **Hallazgo verificado**: la feature `test-failpoints` está declarada
  (`crates/lodestar-workspace/Cargo.toml:39`) pero **ningún fichero de `src/` la referencia**: no
  existe el seam de producción. `simular_caida` (`transactions.rs:1317`) compone las primitivas a
  mano y **en orden invertido** respecto a producción:

  | | orden de pasos |
  |---|---|
  | Producción (`transaction.rs:149-152`) | staging → **backup (8)** → **journal (9)** → renames |
  | Simulación (`transactions.rs:1317-1336`) | staging → **journal** → **backup** → renames |

  Consecuencias: `FailPoint::TrasJournalPrepared` («journal `prepared`, aún sin copias de
  recuperación») describe un estado que el código real **no puede producir**, y pasa vacuamente
  (sin directorio de recuperación, `restore_from_recovery` devuelve `Ok(())` de inmediato,
  `recovery.rs:315`). El estado que el código real **sí** produce —backup escrito, journal aún
  ausente— **no está en la taxonomía**. El propio `recover()` documenta la invariante de producción
  que la simulación invierte.
- **Referencias**: `crates/lodestar-workspace/src/transaction.rs:100` ·
  `crates/lodestar-workspace/tests/transactions.rs:1151` (`mod recuperacion`).
- **Alcance**:
  - Introducir el punto de aborto **dentro de `apply_transaction`**, bajo
    `#[cfg(feature = "test-failpoints")]`.
  - Corregir la taxonomía `FailPoint` al orden real y añadir el estado que falta.
  - Reapuntar `recovery_sin_parciales` al orquestador real en vez de a `simular_caida`.
- **Criterios de aceptación**:
  - **Dado** cada punto de caída, **Cuando** se aborta **dentro de `apply_transaction`** y se
    recupera, **Entonces** el canónico converge a uno de los dos bordes, jamás a un parcial →
    `recovery_sin_parciales` (reapuntado).
  - **Dado** el estado «backup escrito, journal ausente», **Cuando** se reabre, **Entonces** el
    comportamiento está fijado por test → `caida_entre_backup_y_journal`.
- **Dependencias**: bloque B cerrado (H03, H05, H06 cambian el camino transaccional).
- **Pruebas**: `crates/lodestar-workspace/tests/transactions.rs`.
- **Frontera (mcp.yml)**: no.

### E24-H14 — El crash por señal, como test permanente

- **Objetivo**: que el escenario 12 de `§17` deje de estar degradado y pruebe un crash de verdad.
- **Referencias**: `crates/lodestar-mcp/tests/benchmark.rs` (`escenario_12_crash_recuperacion`, hoy
  solo prueba durabilidad tras cerrar/reabrir) · el sondeo de la revisión, que ejecutó 30 `SIGKILL`
  reales y destapó D2, D3 y D5.
- **Alcance**: `SIGKILL` al binario a retrasos escalonados durante `change_apply`; verificar que
  ningún `.md` queda a medias, que el canónico converge a un borde y —tras el bloque B— que la
  siguiente transacción **funciona al primer intento** y no deja fugas en `runtime/`.
- **Criterios de aceptación**:
  - **Dado** un apply interrumpido por señal a distintos retrasos, **Cuando** se reabre y se
    recupera, **Entonces** no hay ni un `.md` a medias y el canónico converge a un borde →
    `crash_por_senal_no_deja_parciales`.
  - **Dado** ese workspace recuperado, **Cuando** se hace la siguiente transacción, **Entonces**
    tiene éxito al primer intento y `runtime/` queda limpio →
    `tras_crash_la_siguiente_transaccion_funciona`.
- **Dependencias**: **E24-H03**, **E24-H05**, **E24-H06**.
- **Pruebas**: `crates/lodestar-mcp/tests/` (arnés de proceso vivo).
- **Frontera (mcp.yml)**: no.

### E24-H15 — `structuredContent` validado contra `outputSchema`

- **Objetivo**: guardia anti-drift entre lo que las tools declaran y lo que emiten.
- **Estado verificado**: hoy **conforma 14/14** con un validador JSON Schema real. Esto **no arregla
  un defecto**: impide uno futuro. `tools_declaran_outputschema` (`mcp.rs`) solo comprueba **5 de las
  10** tools y solo que el schema «tenga alguna clave estructural».
- **Alcance**: validar la salida real de las **10** tools contra su `outputSchema` declarado, con un
  validador de JSON Schema como dev-dependency.
- **Criterios de aceptación**:
  - **Dado** una llamada con éxito a cada una de las 10 tools, **Cuando** se valida su
    `structuredContent` contra su `outputSchema`, **Entonces** conforma →
    `structured_content_conforma_output_schema`.
  - **Dado** un `outputSchema` y una salida deliberadamente incoherentes, **Cuando** se valida,
    **Entonces** el test falla → control anti-vacuo del propio validador.
- **Dependencias**: **E24-H11** (cambia el `outputSchema` de `knowledge_get`).
- **Pruebas**: `crates/lodestar-mcp/tests/mcp.rs`.
- **Frontera (mcp.yml)**: no.

### E24-H16 — Arnés compartido y escala por el wire

- **Objetivo**: que los tests e2e ejerzan lo que un agente real ejerce.
- **Hallazgo verificado**: **5 de los 7** arneses MCP usan **proceso frío** (una tanda de líneas → una
  tanda de respuestas); solo `e2e_ciclo_vida.rs::Sesion` y `descubribilidad.rs::Sesion` mantienen el
  proceso vivo, que es lo único que caza bugs de invalidación de estado — la razón por la que existe
  `E23-H07`. Hay **4 implementaciones duplicadas** del arnés y no existe `tests/common/`. Además,
  escala, auditoría, planes persistidos, selecciones y política de escritura **solo** se ejercen por
  `App` en proceso: los 10k documentos de `app/tests/escala.rs` nunca han pasado por JSON-RPC.
- **Alcance**: un `Sesion` compartido, y una medición de escala por el wire.
- **Criterios de aceptación**:
  - **Dado** el arnés compartido, **Cuando** se compilan los tests de `lodestar-mcp`, **Entonces**
    no quedan implementaciones duplicadas del arnés → revisión de diff.
  - **Dado** un workspace de ~10k documentos, **Cuando** se le hace `knowledge_search` por JSON-RPC,
    **Entonces** ningún cuerpo completo viaja en la respuesta → `escala_por_el_wire_acota_payload`.
- **Dependencias**: ninguna.
- **Pruebas**: `crates/lodestar-mcp/tests/`.
- **Frontera (mcp.yml)**: no.

### E24-H17 — Las afirmaciones sin respaldo pasan a tenerlo

- **Objetivo**: que dos afirmaciones que el repo da por ciertas **existan**.
- **Hallazgos verificados**:
  1. **`schema_declara_todos_los_parametros` nunca se escribió.** Es criterio de aceptación literal de
     `E23-H10` (`requirements/epica-23-cierre-migracion.md:342`), historia que el ledger da por
     cerrada. La cadena aparece **una sola vez en todo el repo** y en ningún `.rs`. El commit
     `c6a9990` tocó solo `contracts/mcp.yml` y `tools.rs`, y su cuerpo sustituye el test por
     «verificado manualmente». Ídem `move_default_explicito`.
  2. **El «grep de CI» que impide redefinir los `ErrorCode` fuera del core no existe.**
     `crates/lodestar-core/src/types.rs:1219` afirma *«está prohibido redefinir estos códigos fuera
     de `core::types` (grep de CI)»*; el CI solo tiene dos greps, ambos sobre `cargo tree` en el job
     `core-purity`. Tampoco hay ningún test que fije que el catálogo tiene 16 filas: el «16» vive
     solo en prosa, en 9 sitios.
- **Alcance**: escribir los tests que faltan y el grep de CI que se afirma.
- **Criterios de aceptación**:
  - **Dado** el `inputSchema` de `change_plan`, **Cuando** se compara con los parámetros que lee
    `normalize_raw_op`, **Entonces** el conjunto declarado ⊇ el leído →
    `schema_declara_todos_los_parametros`.
  - **Dado** el catálogo de `ErrorCode`, **Cuando** corre el CI, **Entonces** falla si algún crate
    fuera de `lodestar-core` define esos literales → nuevo paso en `.github/workflows/ci.yml`.
  - **Dado** el catálogo, **Cuando** corre la suite, **Entonces** un test fija que tiene 16 filas →
    `catalogo_de_errores_tiene_dieciseis_filas`.
- **Dependencias**: **E24-H09**, **E24-H10** (fijan la superficie que estos tests describen).
- **Pruebas**: `crates/lodestar-mcp/tests/descubribilidad.rs`, `crates/lodestar-core/tests/core.rs`,
  `.github/workflows/ci.yml`.
- **Frontera (mcp.yml)**: no.

---

## Bloque G — Documentos y publicación

### E24-H18 — Los documentos dejan de mentir, y se publica v0.3.1

- **Objetivo**: que el estado escrito coincida con el real, y publicar.
- **Drift verificado**:
  - `CLAUDE.md:45` y `ARCHITECTURE.md:932`/`:972` siguen listando **`schema_inspect`** entre las 10
    tools. Murió en `E20-H03`; la real es `metadata_inspect`. `CLAUDE.md` se contradice a sí mismo:
    su línea 28 ya lo dice bien.
  - `IMPLEMENTATION_STATUS.md:27` dice **372 tests**; el recuento real es **437** (la línea 11
    acierta).
- **Alcance**:
  - Corregir el drift anterior.
  - `IMPLEMENTATION_STATUS.md` gana la sección de E24; `requirements/README.md` y
    `requirements/trazabilidad.md` incorporan la épica.
  - `decisiones/` gana un fichero nuevo: **rechazar parámetros no declarados** en la superficie
    MCP — contexto, la política vigente citada, opciones y recomendación, **sin tomar la decisión**.
  - `CHANGELOG.md`: sección `## [0.3.1] - AAAA-MM-DD` con `### Corregido`, viñetas que abren en
    negrita con el síntoma (estilo E23), y pie de enlaces actualizado
    (`[No publicado]: …compare/v0.3.1...HEAD` + `[0.3.1]: …compare/v0.3.0...v0.3.1`).
  - **Avisos de cambio de veredicto** que la 0.3.1 introduce sobre bases existentes, recogidos de
    las historias que los declararon: `lodestar check` puede pasar de exit 0 a exit 1 en un
    workspace con un `.md` con BOM y frontmatter ilegible (E24-H01), y una consulta con una
    propiedad inexistente bajo namespace reservado pasa de devolver `[]` a fallar (E24-H07). Los
    dos son correcciones, pero cambian resultados observables: van en la nota de release.
  - `./scripts/set-version.sh 0.3.1 && cargo update -w`.
- **Criterios de aceptación**:
  - `grep -rn "schema_inspect" CLAUDE.md ARCHITECTURE.md` no devuelve ninguna línea que la presente
    como tool vigente.
  - El recuento de tests de `IMPLEMENTATION_STATUS.md` coincide con
    `cargo test --workspace -- --list | grep -c ": test$"`.
  - `CHANGELOG.md` tiene la sección `0.3.1` con los enlaces de comparación al pie.
  - `Cargo.toml` declara `0.3.1` y `Cargo.lock` está propagado.
- **Dependencias**: **todas las anteriores**.
- **Pruebas**: las puertas del CI completas (incluida la de `--features test-failpoints`).
- **Frontera (mcp.yml)**: no.

---

## Orden de construcción

**v0.3.1** (16 historias):

```
A: H01 ─→ H02                          ┐
B: H03 ─→ H04     ·     H05 ─→ H06     ├─→ F: H13 · H14 · H15 · H16 · H17 ─→ G: H18
D: H09 ─→ H10     ·     H11            │
E: H12                                 ┘
```

**v0.4.0** (después de publicar v0.3.1): `C: H07 ─→ H08`.

Los bloques A, B, D y E son independientes entre sí y paralelizables. Dentro de cada uno el orden es
estricto. El bloque F va después porque sus tests describen el comportamiento ya corregido: **H13**
depende del bloque B entero, **H14** de H03/H05/H06, **H15** de H11, **H17** de H09/H10. **H18**
cierra.

## Proceso por historia

Dos velocidades, según lo que arriesga cada historia (`docs/WORKFLOWS.md §6` contempla la receta
corta para defectos acotados):

| Ciclo | Historias | Por qué |
|---|---|---|
| **Completo** (spec → roja → verde → juez ciego con *mutation testing*) | H01 ✅ · H02 · H03 · H04 · H06 · H09 · H10 · H11 · H13 | Tocan corrupción de datos, el motor transaccional o la forma del wire |
| **Corto** (regresión en rojo → fix → verificación) | H05 · H12 · H14 · H15 · H16 · H17 | Defectos acotados, o puramente aditivas de tests |

**Lección de H01, aplicable a todos los jueces que queden**: los seis tests de su fase roja pasaban
con dos mutaciones distintas del arreglo puestas. El juez lo destapó haciendo *mutation testing* por
su cuenta. A partir de aquí se le pide explícitamente en el encargo.

## Criterio de salida

Un `.md` con BOM conserva su metadata y avisa; tras un crash la siguiente transacción funciona al
primer intento y el workspace no se presenta como roto; ninguna transacción fallida deja staging y el
plano de control no crece sin cota; una consulta con una propiedad que no existe **falla** en vez de
responder vacío, y el frontmatter propio del usuario es siempre alcanzable; los valores de los
parámetros declarados se validan y todo error lleva código del catálogo; `knowledge_get` devuelve
`title`; el crash se prueba abortando el orquestador real y por señal; los dos tests que E23 dio por
escritos existen; y los documentos de estado dicen la verdad. **v0.3.1 publicada** por el runbook de
`RELEASING.md`.
