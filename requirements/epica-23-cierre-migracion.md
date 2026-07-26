# E23 — Cierre de la migración: defectos, puerta de calidad y superficie

> **Fase**: posterior a `§20.14` PR 11. No es una fase del plan de migración: es la **épica de
> cierre** que salda lo que la revisión de la PR #17 destapó antes de mergear.
> **Objetivo de la épica**: que v0.3.0 salga sin defectos conocidos en el camino de escritura, con la
> puerta de CI cubriendo la garantía nuclear del producto, con una superficie MCP que un agente pueda
> usar leyendo solo el schema, y con los documentos de estado diciendo la verdad.
> Referencias maestras: `ARCHITECTURE.md §20` (entero), `docs/REFACTOR_PHASE_2.md`, `DECISIONES.md`.

**Origen**: revisión de la PR #17 (2026-07-25). Los defectos del bloque A **no se dedujeron leyendo
código**: se reprodujeron ejecutando los binarios `lodestar` y `lodestar-mcp` contra workspaces de
prueba. Cada historia del bloque A lleva el síntoma reproducible que la motiva.

**Principio rector**: `ARCHITECTURE.md §20.2` invariante 3 — *"el frontmatter **nunca** es
obligatorio y sus claves **no** tienen semántica impuesta"* — y el invariante #3 de `CLAUDE.md` —
*"una sola verdad computada"*. Los dos están hoy incumplidos en producción.

**Fuera de alcance (explícito)**: construir la CLI como gestor de KB. E23-H15 escribe la **propuesta
de diseño**; la implementación se planifica vía `/planificar` en una PR posterior.

---

## Bloque A — Defectos

### E23-H01 — Una sola verdad de validación

- **Objetivo**: `lodestar check` y `knowledge_check` emiten el **mismo veredicto** sobre el mismo
  workspace.
- **Síntoma reproducible**: con `.lodestar/config.yaml` → `validation: {danglingDocumentLinks:
  ignore, malformedFrontmatter: ignore}` sobre un workspace con un enlace roto y un YAML ilegible:
  `lodestar check` imprime `NO CONFORME` y sale **1**; `knowledge_check` responde `conformant: true`
  con `summary.errors: 0`.
- **Referencias**: `CLAUDE.md` invariante #3 · `ARCHITECTURE.md §20.9` · `crates/lodestar-app/src/lib.rs`
  (`full_analysis` vs `knowledge_check`) · `crates/lodestar-workspace/src/lib.rs`
  (`document_set_with_discovery`).
- **Alcance**:
  - `App::full_analysis()` deja de ir por `document_set().analyze()` a secas y pasa por el mismo
    camino que `knowledge_check`: aplica `ValidationSection::effective_severity` (la sección
    `validation` de la config) y añade los diagnósticos de descubrimiento (`DOC-NOT-UTF8`,
    `DOC-TOO-LARGE`, `SYMLINK-UNSUPPORTED`, `PATH-NOT-UTF8`, `LINK-CASE-MISMATCH`).
  - Reusar `Workspace::document_set_with_discovery()`, que ya devuelve ambas mitades. **No** duplicar
    la lógica de severidad: extraer un único punto si hace falta.
  - `commands.rs` no cambia de forma: sigue derivando `conformant` de la ausencia de `Err`, pero
    sobre el análisis correcto.
- **Criterios de aceptación**:
  - **Dado** un workspace con un enlace roto y `validation: {danglingDocumentLinks: ignore}`,
    **Cuando** se corre `lodestar check` y se llama a `knowledge_check(scope: workspace)`,
    **Entonces** ambos dicen conforme (exit 0 / `conformant: true`) →
    `check_y_knowledge_check_coinciden_con_ignore`.
  - **Dado** ese mismo workspace con `validation: {danglingDocumentLinks: error}`, **Cuando** se
    corre lo mismo, **Entonces** ambos dicen NO conforme (exit 1 / `conformant: false`) →
    `check_y_knowledge_check_coinciden_con_error` (control anti-vacuo: la coincidencia no puede ser
    "los dos dicen siempre que sí").
  - **Dado** un workspace con un `.md` no UTF-8, **Cuando** se corre `lodestar check --json`,
    **Entonces** el diagnóstico de descubrimiento aparece en la salida →
    `check_ve_diagnosticos_de_descubrimiento`.
- **Dependencias**: ninguna.
- **Pruebas**: `crates/lodestar-app/tests/validacion.rs` + `crates/lodestar-cli/tests/cli.rs`.
- **Frontera (mcp.yml)**: no (cambia el cómputo, no la forma del wire).

### E23-H02 — `create` sin residuo OKF

- **Objetivo**: crear un documento **no** inyecta claves de frontmatter que el usuario no pidió.
- **Síntoma reproducible**: `change_plan` con `{"op":"create","path":"notas/nueva.md","body":"# Nueva"}`
  seguido de `change_apply` escribe en disco:
  ```
  ---
  title: nueva
  type: ''
  ---
  ```
  Un `type` vacío heredado de OKF, en un producto cuyo invariante 3 dice que las claves del
  frontmatter no tienen semántica impuesta.
- **Referencias**: `ARCHITECTURE.md §20.2` invariante 3, `§20.4` · `crates/lodestar-core/src/plan.rs`
  (`normalize_create`) · `crates/lodestar-app/src/lib.rs` (`normalize_raw_op`, rama `"create"`).
- **Alcance**:
  - `plan::normalize_create` deja de insertar `type` y `title`. Firma nueva: acepta un
    `frontmatter: Option<FrontmatterPatch>` arbitrario (lo que el llamador pida, tal cual) y un
    `body: Option<String>`.
  - `normalize_raw_op` deja de leer `type`/`title` como campos privilegiados y pasa el objeto
    `frontmatter` de la op. Un `create` sin `frontmatter` produce un `.md` **sin bloque de
    frontmatter** (no un bloque vacío).
  - Declarar `frontmatter`/`body` en el `inputSchema` (coordinado con E23-H10).
  - **Migración de tests**: ~15 tests pasan `"type"` a `create` (`mcp.rs`, `benchmark.rs`,
    `escala.rs`, `escritura.rs`, `validacion.rs`, `sin_regen.rs`). Se actualizan como parte del
    cambio de spec; donde el `type` sea incidental, se retira; donde importe, pasa a `frontmatter`.
- **Criterios de aceptación**:
  - **Dado** un `create` sin `frontmatter`, **Cuando** se aplica, **Entonces** el `.md` en disco
    **no** contiene `type:` ni `title:` ni un bloque `---` vacío → `create_sin_frontmatter_no_inyecta`.
  - **Dado** un `create` con `frontmatter: {estado: "borrador", tags: [a, b]}`, **Cuando** se aplica,
    **Entonces** el `.md` lleva exactamente esas claves con sus tipos YAML → `create_frontmatter_arbitrario`.
  - **Dado** el documento creado sin frontmatter, **Cuando** se pide `knowledge_get`, **Entonces** el
    título derivado sale del primer H1 o del nombre del fichero (`§20.4`) →
    `create_sin_frontmatter_titulo_derivado`.
- **Dependencias**: ninguna (coordina con H10 en el schema).
- **Pruebas**: `crates/lodestar-core/tests/core.rs` + `crates/lodestar-mcp/tests/mcp.rs`.
- **Frontera (mcp.yml)**: **sí** — cambian los parámetros de la op `create`.

### E23-H03 — `move` recalcula sus propios enlaces salientes

- **Objetivo**: mover un documento que enlaza a sus vecinas deja el workspace consistente.
- **Síntoma reproducible**: con `notas/alfa.md` que contiene `[b]: beta.md` y `notas/beta.md`
  existente, `change_plan` con `{"op":"move","from":"notas/alfa.md","to":"archivo/alfa.md",
  "rewriteInboundLinks":true}` devuelve `canApply: false` con `diagnosticsAfter.errors: 1` y el apply
  falla. **Control**: mover un documento sin enlaces salientes funciona y reescribe correctamente los
  entrantes (incluidas las definiciones de referencia), o sea que el defecto está acotado a los
  salientes del documento movido.
- **Referencias**: `ARCHITECTURE.md §20.11` (`move_document`) · `crates/lodestar-core/src/plan.rs`
  (`normalize_move`, `rewrite_body_links`, `retarget_body_links`, `LinkAction`).
- **Alcance**:
  - `normalize_move` emite, además del `Move` y de los `ReplaceBody` de los emisores entrantes, un
    `ReplaceBody` sobre el **propio documento movido** con sus hrefs relativos recalculados desde la
    ubicación nueva.
  - Reusar la maquinaria por `span` que ya existe (la misma que cubre las definiciones de
    referencia). **No** tocar: URIs externas, anchors propios (`#seccion`), hrefs raíz-absolutos
    (`/docs/x.md`) ni destinos que ya sean correctos desde la ubicación nueva.
  - Un enlace saliente **roto antes** del move sigue roto después (no se inventa destino): el gate
    diferencial de E20-H04 lo permite porque no es un error *nuevo*.
- **Criterios de aceptación**:
  - **Dado** `notas/alfa.md` con un enlace inline y una definición de referencia a `notas/beta.md`,
    **Cuando** se mueve a `archivo/alfa.md` y se **aplica**, **Entonces** ambos hrefs quedan
    `../notas/beta.md` y `lodestar check` sale 0 → `move_recalcula_salientes` (**el test aplica, no
    solo planifica**).
  - **Dado** ese documento con además un `https://…`, un `#anchor` y un `/raiz.md`, **Cuando** se
    mueve, **Entonces** los tres quedan **intactos** byte a byte → `move_no_toca_externos_ni_anchors`.
  - **Dado** un documento con 30 backlinks entrantes y enlaces salientes propios, **Cuando** se mueve
    y se aplica, **Entonces** los 30 emisores apuntan al destino nuevo y el documento movido conserva
    sus salientes válidos, en una sola transacción → `move_completo_treinta_backlinks`.
  - **Dado** ese move aplicado, **Cuando** se hace `change_revert`, **Entonces** los 32 ficheros
    vuelven a su contenido previo → `move_revert_completo`.
- **Dependencias**: ninguna. **Habilita** el paso `move` de E23-H07.
- **Pruebas**: `crates/lodestar-core/tests/core.rs` + `crates/lodestar-mcp/tests/benchmark.rs`
  (ampliar `escenario_04`, que hoy solo planifica).
- **Frontera (mcp.yml)**: no (misma op, más ops normalizadas).

### E23-H04 — `pendingTransaction` real

- **Objetivo**: tras un crash, la primera tool que llama un agente le dice la verdad.
- **Síntoma**: `workspace_status.recovery.pendingTransaction` es un `false` literal en
  `crates/lodestar-app/src/lib.rs`, pese a que `Workspace::recovery_pending()` existe y funciona
  desde E13-H06. El agente planifica normalmente y solo descubre el problema cuando `change_apply`
  explota con `WORKSPACE_RECOVERY_REQUIRED`.
- **Referencias**: `crates/lodestar-workspace/src/recovery.rs` (`recovery_pending`) ·
  `contracts/mcp.yml` (nota «siempre `false` hasta E13-H06», obsoleta desde hace tres épicas).
- **Alcance**: cablear el campo. Actualizar el doc-comment de `StatusRecovery` y la nota del
  contrato. Es el sexto hueco de cableado de la misma familia (E17 `other_files`, E20-H04
  diagnósticos, E22-H04 selección masiva): **el criterio de aceptación exige un test por la frontera
  MCP, no por la capa `App`**.
- **Criterios de aceptación**:
  - **Dado** un workspace con una transacción a medias (journal presente, vía failpoint o montaje
    directo del árbol de recovery), **Cuando** se llama a `workspace_status` por MCP, **Entonces**
    `recovery.pendingTransaction` es `true` → `status_reporta_recovery_pendiente`.
  - **Dado** un workspace limpio, **Cuando** se llama a `workspace_status`, **Entonces** es `false`
    (control anti-vacuo) → `status_sin_recovery_pendiente`.
- **Dependencias**: se apoya en E23-H06 si el montaje usa failpoints.
- **Pruebas**: `crates/lodestar-mcp/tests/mcp.rs`.
- **Frontera (mcp.yml)**: sí (solo la nota semántica; la forma no cambia).

### E23-H05 — Políticas de borrado honestas

- **Objetivo**: `delete_document` no acepta una política que no ejecuta.
- **Síntoma reproducible**: `delete` con `inboundLinksPolicy: "retarget"` sobre un documento con un
  backlink produce un plan con **solo** el borrado; el gate lo rechaza con `diagnosticsAfter.errors:
  1` («introduciría un error»), que no es la verdad: la verdad es que la política no está
  implementada. Igual con `create_stub`. Documentado como «implementación mínima» en
  `crates/lodestar-core/src/plan.rs` desde E12-H06, y E13/E14/E21-H03 pasaron sin cerrarlo.
- **Referencias**: `ARCHITECTURE.md §20.11` (delete con política explícita) ·
  `docs/REFACTOR_PHASE_2 §Fase 12` · `contracts/mcp.yml` (valores de `inboundLinksPolicy`).
- **Alcance**: **decisión de la historia** — implementar las dos, o retirarlas. Recomendación:
  **retirarlas** de `InboundLinksPolicy`, del `inputSchema` y del contrato, dejando `reject` y
  `remove_links`. `retarget` no tiene siquiera campo donde indicar el destino, y `create_stub` no
  tiene criterio de contenido; inventar semántica aquí sería peor que no ofrecerla. Un valor retirado
  pasa a ser `INVALID_SCHEMA` con mensaje explícito.
- **Criterios de aceptación**:
  - **Dado** un `delete` con `inboundLinksPolicy: "retarget"`, **Cuando** se planifica, **Entonces**
    se rechaza con `INVALID_SCHEMA` y un mensaje que nombra las políticas válidas → `delete_retarget_rechazado`.
  - **Dado** un `delete` con `inboundLinksPolicy: "remove_links"` sobre un documento con 2 backlinks,
    **Cuando** se **aplica**, **Entonces** el `.md` desaparece, los 2 emisores conservan el texto sin
    el enlace, y `check` sale 0 → `delete_remove_links_aplicado`.
  - **Dado** ese borrado aplicado, **Cuando** se hace `change_revert`, **Entonces** el documento
    vuelve **byte a byte** y los emisores recuperan sus enlaces → `delete_revert_byte_a_byte`.
- **Dependencias**: ninguna.
- **Pruebas**: `crates/lodestar-app/tests/eliminacion.rs` + `crates/lodestar-mcp/tests/benchmark.rs`.
- **Frontera (mcp.yml)**: **sí** — cambia el enum de `inboundLinksPolicy`.

---

## Bloque B — Puerta de calidad y tests e2e

### E23-H06 — El CI corre los failpoints

- **Objetivo**: la garantía nuclear del producto («un crash no deja el canónico a medias») tiene
  puerta.
- **Síntoma**: los 4 tests de `#[cfg(feature = "test-failpoints")] mod recuperacion`
  (`crates/lodestar-workspace/tests/transactions.rs`) **no se ejecutan jamás**: `ci.yml` corre
  `cargo test --workspace --locked` sin la feature. `recovery_sin_parciales` es, por declaración del
  propio `benchmark.rs`, la prueba autoritativa de no-corrupción, y `recovery_bloquea_escritura` es
  el único test del repo que asevera `WORKSPACE_RECOVERY_REQUIRED`.
- **Alcance**: un step nuevo en el job `rust` de `.github/workflows/ci.yml`:
  `cargo test -p lodestar-workspace --features test-failpoints --locked`. Documentarlo en la sección
  «Build, test y lint» de `CLAUDE.md` y en el README.
- **Criterios de aceptación**:
  - **Dado** el workflow, **Cuando** se inspecciona, **Entonces** existe un step que pasa
    `--features test-failpoints` → verificación por checklist + `act`/lectura.
  - **Dado** el comando del step, **Cuando** se ejecuta en local, **Entonces** los 4 tests corren y
    pasan → ejecución real.
- **Dependencias**: ninguna. **Va primero**: abre la puerta antes de tocar el motor transaccional.
- **Pruebas**: el propio CI.
- **Frontera (mcp.yml)**: no.

### E23-H07 — e2e del ciclo de vida en una sola sesión

- **Objetivo**: probar el patrón de uso real de un agente — una sesión larga con estado vivo.
- **Síntoma**: `e2e_migracion.rs` dice «todo en la misma sesión» pero **cada paso levanta un proceso
  nuevo**. Solo 5 tests en todo `mcp.rs` envían más de 2 líneas por sesión. Consecuencia: toda
  llamada arranca con estado frío y **la clase entera de bugs de invalidación queda enmascarada**.
- **Alcance**: fichero nuevo `crates/lodestar-mcp/tests/e2e_ciclo_vida.rs`, reusando el arnés `mcp()`
  de `e2e_migracion.rs` pero con **una única invocación** y N líneas JSON-RPC:
  `create → apply → patch_frontmatter → apply → move → apply → delete → apply → revert → revert`,
  verificando el disco y el `workspaceRevision` entre pasos. Incluir una **edición externa** del
  `.md` entre dos llamadas y fijar la semántica esperada (la lectura siguiente la ve, o no la ve, pero
  de forma determinista y documentada).
- **Criterios de aceptación**:
  - **Dado** una sola sesión MCP, **Cuando** se recorre el ciclo completo, **Entonces** cada paso deja
    el disco en el estado esperado y `workspaceRevision` cambia en cada apply →
    `ciclo_vida_una_sesion`.
  - **Dado** un `.md` editado con `std::fs::write` entre dos `tools/call` de la misma sesión,
    **Cuando** se lee después, **Entonces** el resultado es el documentado (y el test lo asevera
    explícitamente, no lo deja al azar) → `edicion_externa_en_sesion_viva`.
  - **Dado** el `delete` aplicado, **Cuando** se revierte, **Entonces** el documento vuelve byte a
    byte → cubre T3.
- **Dependencias**: E23-H03 y E23-H05 (el ciclo incluye move y delete).
- **Pruebas**: `crates/lodestar-mcp/tests/e2e_ciclo_vida.rs` (nuevo).
- **Frontera (mcp.yml)**: no.

### E23-H08 — Cobertura de `reindex` y del descubrimiento por fachada

- **Objetivo**: cerrar los dos huecos más baratos y más visibles.
- **Síntoma**: (1) `reindex` **no tiene ni un test** que lo invoque —solo aparece en la aserción del
  `--help`— siendo 1 de los 3 subcomandos del producto (hueco declarado como «Pendiente» en el ledger
  desde E15). (2) `.gitignore`/`.lodestarignore` se prueban a fondo a nivel workspace pero **nunca
  por una fachada**, siendo la promesa central del refactor (apuntar a un repo real con
  `node_modules/`).
- **Alcance**:
  - `crates/lodestar-cli/tests/cli.rs`: `reindex` crea `.lodestar/index.db` y sale 0; es idempotente
    (correrlo dos veces da el mismo resultado observable); se recupera de un `index.db` corrupto.
  - `crates/lodestar-mcp/tests/e2e_migracion.rs`: añadir a `proyecto_arbitrario` un `.gitignore` con
    `vendor/` y un `vendor/basura.md`, y aseverar que `counts.documents` no cambia y que
    `knowledge_search` no lo devuelve.
  - Unificar el arnés de la CLI en `tempfile::tempdir()` (hoy usa `std::env::temp_dir()` con nombres
    por PID y **nunca limpia**).
- **Criterios de aceptación**:
  - **Dado** un workspace sin cache, **Cuando** se corre `lodestar reindex`, **Entonces** existe
    `.lodestar/index.db` y el exit es 0 → `reindex_crea_cache`.
  - **Dado** una cache recién creada, **Cuando** se corre `reindex` otra vez, **Entonces** exit 0 y
    el `check` posterior es idéntico → `reindex_es_idempotente`.
  - **Dado** un `index.db` con bytes basura, **Cuando** se corre `reindex`, **Entonces** exit 0 y la
    cache queda usable → `reindex_sobre_cache_corrupta`.
  - **Dado** un proyecto con `vendor/` en `.gitignore` y un `.md` dentro, **Cuando** se pregunta a la
    superficie MCP, **Entonces** ni `counts` ni `knowledge_search` lo incluyen →
    `gitignore_respetado_por_la_fachada`.
- **Dependencias**: ninguna.
- **Pruebas**: `crates/lodestar-cli/tests/cli.rs`, `crates/lodestar-mcp/tests/e2e_migracion.rs`.
- **Frontera (mcp.yml)**: no.

### E23-H09 — Bordes

- **Objetivo**: cubrir los bordes que hoy separan «funciona en un tempdir limpio» de «funciona en un
  repo real».
- **Alcance y criterios**:
  - **Concurrencia entre procesos**: hoy solo se prueba entre hilos (`escala.rs`), pero el lock es
    `O_CREAT|O_EXCL`, una primitiva **inter-proceso**, y el despliegue real son N servidores MCP o un
    MCP + un `check` en CI. **Dado** dos procesos aplicando planes que tocan el mismo fichero,
    **Entonces** exactamente uno da `applied: true` y el otro un error de la familia
    `{WRITE_CONFLICT, PLAN_STALE}` → `dos_procesos_un_ganador`. Más: **Dado** un `.lodestar/lock` de
    un PID inexistente, **Entonces** el comportamiento es el documentado (no un bloqueo eterno) →
    `lock_huerfano`.
  - **Unicode en rutas**: cero cobertura hoy. El CI corre en macOS (APFS normaliza a NFD) y en Linux
    (bytes crudos): un `[x](café.md)` en NFC contra un fichero en NFD es exactamente el bug que este
    repo está destinado a sufrir. **Dado** un documento `café.md` enlazado desde otro, **Entonces** el
    enlace resuelve en las 3 plataformas → `unicode_en_rutas`.
  - **`patch_frontmatter` sobre YAML ilegible**: la vía más directa a pérdida de datos, sin probar en
    el camino de escritura. **Dado** un documento con `FM-YAML-INVALID`, **Cuando** se le aplica un
    `patch_frontmatter`, **Entonces** falla limpio **sin** destruir el bloque original →
    `patch_sobre_frontmatter_ilegible`.
  - **Códigos sin emisor**: `AMBIGUOUS_REFERENCE`, `RESULT_TOO_LARGE`, `RECOVERY_FAILED`,
    `INTERNAL_IO_ERROR` no aparecen en ningún test del catálogo **congelado** de 16 filas. Cubrir los
    que sí tienen camino alcanzable (empezando por `WORKSPACE_NOT_FOUND` con `--root` inexistente) y,
    para los que no lo tengan, registrarlo explícitamente en el contrato.
  - **Trocear `e2e_migracion.rs`**: hoy es un `#[test]` de 350 líneas con 7 fases; si falla la fase 2,
    el informe de CI dice solo «flujo_completo_migracion failed». Un `#[test]` por fase más un
    agregado, como ya hace bien `benchmark.rs`.
- **Dependencias**: E23-H06 para lo que use failpoints.
- **Pruebas**: `crates/lodestar-mcp/tests/`, `crates/lodestar-workspace/tests/discovery.rs`.
- **Frontera (mcp.yml)**: no.

---

## Bloque C — Superficie MCP

### E23-H10 — El schema de escritura deja de ser opaco

- **Objetivo**: un cliente MCP que lea **solo** el `inputSchema` puede usar las 8 operaciones.
- **Síntoma**: `change_plan` declara únicamente `op`/`path`/`ref`/`expectedRevision` y **no documenta
  ni uno** de los parámetros reales de 7 de las 8 ops: `frontmatter`/`body` (create), `patch`,
  `find`/`replace`/`expectedOccurrences`, `headingPath`/`mode`/`content`,
  `from`/`to`/`rewriteInboundLinks`, `inboundLinksPolicy`, `fixId`. Para un producto cuyo **público
  objetivo son agentes**, es el mayor agujero de usabilidad de la superficie.
- **Referencias**: `crates/lodestar-mcp/src/tools.rs` (`change_plan.inputSchema`) ·
  `crates/lodestar-app/src/lib.rs` (`normalize_raw_op`, la fuente de verdad de los nombres) ·
  `contracts/mcp.yml`.
- **Alcance**: declarar el schema por op (`oneOf` discriminado por `op`, o propiedades documentadas
  con la condición en la descripción — decidir en la historia), con los nombres **exactos** que lee
  `normalize_raw_op`. Sincronizar con `/contrato`. **Además**: `rewriteInboundLinks` tiene default
  `false`, o sea que **el default rompe la KB en silencio**; invertirlo o exigirlo explícito, como ya
  se hace con `inboundLinksPolicy`.
- **Criterios de aceptación**:
  - **Dado** el `inputSchema` de `change_plan`, **Cuando** se comparan sus propiedades con las que lee
    `normalize_raw_op`, **Entonces** no falta ninguna → `schema_declara_todos_los_parametros` (test
    que recorre las 8 ops, no una aserción de subcadena).
  - **Dado** un `move` sin `rewriteInboundLinks`, **Cuando** se planifica, **Entonces** el
    comportamiento es el decidido y está declarado en el schema → `move_default_explicito`.
  - **Dado** `/contrato --check`, **Cuando** se ejecuta, **Entonces** no reporta drift bloqueante.
- **Dependencias**: E23-H02 y E23-H05 (cambian parámetros y enums que este schema declara).
- **Pruebas**: `crates/lodestar-mcp/tests/mcp.rs` + inline de `tools.rs`.
- **Frontera (mcp.yml)**: **sí**.

### E23-H11 — Descubribilidad de la KB

- **Objetivo**: que un agente pueda entender una base de notas ajena sin pagar N+1 llamadas.
- **Síntomas verificados**:
  - `metadata_inspect(mode:"field", field:"tags")` devuelve `values: []` con `inferredTypes:
    {list: 3}`: **no explota las listas**, así que es imposible obtener el vocabulario de tags de una
    base — el caso de uso número uno de una KB de notas, y justo lo que la tool promete hacer.
  - `knowledge_search` devuelve solo `path`/`title`/`snippet`/`score`/`revision`: **ni un campo de
    frontmatter**. Ver el `status` de 30 resultados cuesta 30 `knowledge_get`. E19-H05 retiró
    `type`/`status`/`tags` sin poner nada genérico en su lugar.
  - `sort` se acepta y **se ignora en silencio** (`_sort` en la firma).
  - `apply_fix` está anunciada como una de las 8 ops en el schema, el contrato y el CHANGELOG, pero
    **siempre falla** (`FixNotFound`): no hay productor de `fixes` desde E20-H03.
  - Un enlace a la raíz (`[x](../)`, `[x](./)`) da `LINK-ESCAPES-WORKSPACE` con severidad **error**,
    o sea que puede tumbar la puerta de CI por un enlace legítimo que GitHub sí resuelve. Deuda
    registrada al cerrar E17 y aplazada a E20/E21, que cerraron sin hacerlo.
  - No hay forma de **listar receipts**: si el agente pierde el `receiptId`, el undo es inalcanzable
    pese a que los receipts están persistidos y hay `audit.jsonl`.
- **Alcance**: `metadata_inspect` explota listas al contar valores; `knowledge_search` acepta una
  proyección de campos de frontmatter pedida por el llamador; `sort` se implementa o se retira del
  schema; `apply_fix` sale del enum hasta que haya productor; `LinkTarget` se amplía para el destino
  «raíz del workspace» (arreglo correcto según el ledger, no parchear el diagnóstico); se decide si
  listar receipts entra aquí o es historia propia.
- **Criterios de aceptación**:
  - **Dado** un workspace con `tags: [a, b]` en 3 documentos, **Cuando** se pide
    `metadata_inspect(field:"tags")`, **Entonces** `values` trae `a` y `b` con sus frecuencias →
    `metadata_inspect_explota_listas`.
  - **Dado** un `knowledge_search` que pide `include: ["frontmatter.status"]`, **Entonces** cada
    resultado trae ese campo → `search_proyecta_frontmatter`.
  - **Dado** un documento con `[volver](../)`, **Cuando** se corre `check`, **Entonces** **no** es un
    error bloqueante → `enlace_a_la_raiz_no_tumba_el_check`.
  - **Dado** un `apply_fix`, **Entonces** la op no está en el enum del schema →
    `apply_fix_retirada`.
- **Dependencias**: E23-H10 (comparten el schema).
- **Pruebas**: `crates/lodestar-core/tests/metadata.rs`, `enlaces.rs`, `crates/lodestar-mcp/tests/mcp.rs`.
- **Frontera (mcp.yml)**: **sí**.

### E23-H12 — Higiene de efectos secundarios

- **Objetivo**: abrir un workspace no modifica el proyecto del usuario.
- **Síntoma verificado**: `Workspace::open` ejecuta `gitignore::ensure_gitignore(root)` y
  `runtime::ensure_runtime_scaffold(root)`, o sea que **tanto `lodestar check` como arrancar el MCP
  reescriben el `.gitignore` del proyecto** y crean `.lodestar/runtime/` antes de leer nada. Para el
  pitch «`cd my-project && lodestar-mcp` sobre cualquier proyecto» es una escritura no solicitada;
  en CI, deja el working tree sucio.
- **Alcance**: no tocar el `.gitignore` en el camino de solo-lectura (o hacerlo opt-in por config);
  `migrate-from-okf` ya demuestra que existe un modo hermético (`open_ephemeral`) reutilizable.
  Además: retirar `implemented_by`/`verified_by` como claves de frontmatter privilegiadas y no
  configurables (`crates/lodestar-workspace/src/external_refs.rs`), último residuo OKF con semántica
  impuesta, contra el invariante 3 de `§20.2`.
- **Criterios de aceptación**:
  - **Dado** un proyecto con un `.gitignore` propio, **Cuando** se corre `lodestar check`,
    **Entonces** el `.gitignore` queda **byte a byte** igual → `check_no_ensucia_el_working_tree`.
  - **Dado** ese proyecto, **Cuando** se arranca el MCP en perfil `readonly`, **Entonces** tampoco
    cambia → `readonly_no_escribe_nada`.
- **Dependencias**: ninguna.
- **Pruebas**: `crates/lodestar-cli/tests/cli.rs`, `crates/lodestar-mcp/tests/mcp.rs`.
- **Frontera (mcp.yml)**: parcial (`externalReferences`).

---

## Bloque D — Documentos de estado

### E23-H13 — Poner al día el ledger

- **Objetivo**: que los documentos de estado dejen de contradecirse.
- **Alcance**:
  - `IMPLEMENTATION_STATUS.md`: la tabla de la migración sigue diciendo **«E15–E22 EN CURSO»** con
    E17–E22 «⚪ Pendiente», 350 líneas por encima del detalle que las da por cerradas. **Era criterio
    de aceptación literal de E22-H03** («marcar la migración E15–E22 como completa»): está
    incumplido. La cabecera describe la UI Tauri, «~113 tests» (son ~381) y los subcomandos git
    `log`/`last-conforming`/`branch`/`switch`/`merge`/`hooks`, borrados en E9-H02 y cuyo crate murió
    en E15-H01. E15-H08 no tiene entrada propia pese a cerrar `DECISIONES §8`.
  - `requirements/README.md`: E20 marcada como *(pendiente)* y sin enlace; el «hueco de cableado
    pendiente, con dueño (E20)» ya lo cerró E20-H04.
  - `CHANGELOG.md` dice `Bundle → DocumentSet` donde `ARCHITECTURE §20.3` dice `Bundle → Workspace`.
  - Doc-comments obsoletos en API pública (viajan en `cargo doc`): `App::open` menciona descubrimiento
    de git e identidad desde `lodestar.toml`, ambos borrados; `core::types` sigue documentando el
    `.d.ts` de ts-rs; `contracts/mcp.yml` dice que `externalReferences` está siempre vacío, falso.
- **Criterios de aceptación**: checklist verificable — ninguna mención viva a la UI Tauri, a git como
  capacidad, ni a subcomandos borrados; la tabla E15–E22 en COMPLETA; el nº de tests real; `grep` de
  `ts-rs`/`lodestar.toml`/`DocumentSet` sin falsos positivos.
- **Dependencias**: se hace **al final**, con el estado ya real.
- **Frontera (mcp.yml)**: sí (notas semánticas).

### E23-H14 — Cerrar o documentar las decisiones abiertas

- **`DECISIONES §12` (fechas)**: las comparaciones son **lexicográficas** porque `serde_yaml` 0.9 no
  tipa timestamps. Se recomendó «(a) para E19 y reevaluar en E20»; E19 y E20 cerraron sin tocarla, y
  la limitación **no está documentada en ninguna superficie de usuario** (ni README ni contrato).
  Es una coerción implícita de facto en un motor que presume de no tener ninguna. **Mínimo
  aceptable**: declararlo por escrito en el contrato y el README, y marcar la decisión como cerrada
  en la opción (a).
- **`DECISIONES §13` (`Conformant → Valid`)**: el **único de los 29 criterios de aceptación** de
  `REFACTOR_PHASE_2` demostrablemente incompleto («no existe terminología OKF en la API pública»:
  sobreviven `conformant`, `requireConformantResult`, `NONCONFORMANT_RESULT`). v0.3.0 ya es
  incompatible, así que es el momento barato de abrir el catálogo de errores **una sola vez**.
- **Criterios de aceptación**: ninguna de las dos decisiones queda en estado ABIERTA/APLAZADA sin una
  línea de resolución fechada; si se cierra §13, `grep -i conformant` sobre la superficie activa no
  devuelve nada.
- **Dependencias**: `DECISIONES §13` toca el catálogo congelado → coordinar con E23-H10.

---

## Bloque F — Defectos que destapó el propio bloque B

> Estas dos historias **no estaban en el plan**: las encontró `E23-H09` al cubrir los bordes, y son
> exactamente la razón por la que esos bordes había que cubrirlos. Ambas se decidieron con el usuario
> antes de tocar código, porque las dos tienen riesgo de diseño.

### E23-H23 — El lock huérfano se reclama; NFC/NFD deja de tumbar el CI

**Defecto 1 — un proceso muerto cerraba el workspace a la escritura para siempre.**
`acquire_lock` solo miraba si `.lodestar/runtime/lock.json` existía. El `pid` que el propio lock
escribe en `lock_metadata()` **nadie lo leía de vuelta**, y no había TTL. Un `lodestar-mcp` muerto
por `SIGKILL` o por el OOM killer dejaba el fichero en disco y la base quedaba inservible para
escribir hasta que un humano lo borrara a mano — y por la frontera MCP el agente solo veía un
`WRITE_CONFLICT` pelado, sin ninguna pista de qué mirar (el mensaje interno sí nombra la ruta, pero
`tools.rs` emite únicamente el código estable).

Arreglo: se reclama si **el dueño ya no existe** (`kill(pid, 0)` → `ESRCH`, solo Unix, inmediato y
exacto) **o** si el lock supera el `LOCK_TTL` (15 min, red portable para Windows). Ante la duda
—fichero ilegible, `pid` ausente, reloj hacia atrás— **no se reclama**: perder disponibilidad es
recuperable, romper el escritor único no. El control anti-vacuo del test es el que de verdad
importa: un lock de un proceso **vivo** no se reclama, porque «reclamar huérfanos» mal implementado
sería «borrar siempre el lock», mucho peor que el defecto.

**Defecto 2 — un enlace correcto tumbaba el CI en macOS.** Lodestar comparaba rutas byte a byte, así
que `café.md` en NFC y en NFD eran dos rutas distintas. En Linux el veredicto era correcto (el
fichero NFD de verdad no existe); en macOS/APFS el fichero **sí se abre** con la otra forma, así que
`LINK-TARGET-MISSING` con severidad `Err` hacía salir `lodestar check` con 1 por un enlace que el SO
y GitHub resuelven. Mismo veredicto en las dos plataformas, acertado solo en una. Disparador
realista: fichero creado por un checkout en macOS, enlace tecleado en Linux.

Arreglo: `fold_path` normaliza a **NFC** además de bajar a minúsculas, o sea que el índice tolerante
del `Inventory` relaciona las dos formas y el diagnóstico degrada a `LINK-CASE-MISMATCH` (`Warn`,
aviso de portabilidad) en vez de a un error bloqueante. **No se normaliza la ruta canónica**, y es la
decisión clave: en Linux el fichero está literalmente en NFD, así que un `RelPath` reescrito a NFC
dejaría de poder abrirlo — sería peor que el bug. El `target` sigue siendo `Missing`, igual que con
la capitalización desde E17-H03: la tolerancia vive en el **diagnóstico**, no en la clasificación.

- **Criterios**: `lock_huerfano` (reclamo + no-reclamo de un vivo) ·
  `unicode_nfc_y_nfd_resuelven_con_aviso` (aviso `Warn`, ni un `Err`, y el `related` señalando la
  ruta real, que es la pista que antes no existía).
- **Pruebas**: `crates/lodestar-mcp/tests/concurrencia.rs`,
  `crates/lodestar-workspace/tests/discovery.rs`.

### E23-H24 — Códigos del catálogo sin emisor (registrar, no inventar)

`E23-H09` verificó que **4 de las 16 filas** del catálogo congelado no tienen productor:
`WORKSPACE_NOT_FOUND` (un `--root` inexistente sale por `std::process::exit(3)` con texto plano,
nunca por el envelope), `RESULT_TOO_LARGE` (mapeado desde `CoreError::SizeGuardExceeded`, variante
que no se construye en ninguna parte), `RECOVERY_FAILED` (cero apariciones fuera de `types.rs`) y
`AMBIGUOUS_REFERENCE` (declarado RESERVADO hasta que exista resolución por `id`).
`INTERNAL_IO_ERROR` sí resultó alcanzable y quedó cubierto.

No se inventan emisores: un código sin camino real es información sobre el catálogo, no un hueco que
haya que rellenar. Se registra en `contracts/mcp.yml` para que la próxima auditoría no lo redescubra.

---

## Bloque E — Documentos nuevos (se escriben aquí, se planifican después)

### E23-H15 — `docs/PROPUESTA_CLI.md`

- **Objetivo**: dejar por escrito la propuesta de diseño para que la CLI sea un gestor de KB, **sin
  implementar nada**, redactada para que `/planificar` la consuma en una PR posterior.
- **Contenido**:
  - **Diagnóstico**: la CLI tiene 3 subcomandos (`check`, `reindex`, `migrate-from-okf`) y **cero**
    capacidad de lectura o escritura de conocimiento; todo el valor vive detrás del MCP. Un humano no
    puede usar su propia KB sin hablar JSON-RPC.
  - **Principio**: paridad de **capacidades**, no de **forma**. `lodestar-app` ya existe como capa de
    casos de uso compartida por las dos fachadas (`ARCHITECTURE §19.2`), así que la lectura son
    shells finos sobre métodos ya escritos y probados.
  - **Lectura propuesta**: `search` (con `where`/`filter`), `get`, `graph`, `status`, `metadata`,
    `impact`, todos con `--json`; `reindex`/`migrate-from-okf` ganan `--json` también.
  - **Escritura propuesta**: **verbos** (`new`, `mv`, `rm`, `set`) que por dentro hacen plan+apply,
    más una escotilla `plan --file / apply / revert` para scripting. Copiar el JSON de operaciones
    del MCP a la línea de comandos sería mala ergonomía para el caso humano.
  - **Riesgo a resolver antes de la escritura**: la CLI escribiendo mientras corre un servidor MCP es
    el caso de concurrencia **entre procesos** que hoy no tiene test (E23-H09). La escritura por CLI
    debe ir condicionada a que esa red esté puesta.
  - **Preguntas abiertas para la puerta de diseño**: ¿la CLI hereda los perfiles
    `readonly`/`standard`? ¿`--path` se renombra a `--root` por simetría con el MCP? ¿salida humana
    por defecto o `--json`? ¿la escritura por CLI respeta `writableRoots`?
- **Criterios de aceptación**: el documento existe, no propone implementación en esta PR, y es
  consumible por `/planificar` (tiene diagnóstico, principio, alcance propuesto y preguntas abiertas).

### E23-H16 — `DECISIONES.md §14`: qué hacemos con el store

- **Objetivo**: registrar como **decisión abierta que requiere criterio del usuario** que la épica
  E18 entera es capacidad construida sin consumidor.
- **Hallazgo verificado**: el único `enable_cache()` del producto está en `lodestar reindex`
  (`crates/lodestar-cli/src/commands.rs`), que solo la **construye**. Ninguna tool MCP lee de SQLite:
  `knowledge_search` usa `core::text::loose_text_match` sobre el `DocumentSet` en RAM, y
  `Workspace::document_set()` **relee y reparsea la KB completa desde disco en cada llamada**. Toda
  la E18 —DDL v2, metadata anidada por field path, FTS genérico, tests de paridad— no llega al
  producto, y las mediciones de escala de E14-H05 son el rendimiento real.
- **Agravante**: el walker del store (`crates/lodestar-store/src/lib.rs`) construye su **propio**
  `WalkBuilder` y **no aplica la `DiscoveryPolicy`** (ni `.lodestarignore`, ni `include`/`exclude`,
  ni el límite de tamaño, ni el endurecimiento de determinismo). E15-H07 difirió esa reconfiguración
  «a E18», y E18 tiene 4 historias y ninguna es esa. Consecuencia: la **paridad core↔store**
  —invariante 13 de `REFACTOR_PHASE_2`— solo se sostiene bajo política por defecto, que es justo el
  caso que prueban los tests de paridad. Hoy es inocua porque nadie lee el store; se vuelve un bug
  real en cuanto se conecte.
- **Opciones a decidir**: (a) conectarlo con invalidación por hash y alinear el walker con
  `DiscoveryPolicy`; (b) declarar que la cache solo sirve a `reindex` y documentarlo; (c) retirarla.
- **Criterios de aceptación**: `DECISIONES.md` gana una sección §14 con contexto, hallazgo,
  agravante, opciones y recomendación — **sin tomar la decisión** (`CLAUDE.md`: «no las tomes por tu
  cuenta: propón y pregunta»).

---

## Orden de construcción

```
H06 (CI failpoints)
  └─→ H01 · H02 · H03 · H04 · H05        (bloque A, paralelizables entre sí)
        └─→ H07 · H08 · H09              (bloque B; H07 depende de H03 y H05)
              └─→ H10 · H11 · H12        (bloque C; H10 depende de H02 y H05)
                    └─→ H13 · H14 · H15 · H16   (documentos, con el estado ya real)
```

## Criterio de salida

`lodestar check` y `knowledge_check` coinciden sobre cualquier config; crear un documento no inyecta
claves que nadie pidió; se puede mover una nota que enlaza a sus vecinas; el estado de recuperación
es real; ninguna política de borrado se acepta sin ejecutarse; el CI corre los tests de crash; existe
un e2e de sesión larga que aplica move y delete; el `inputSchema` de escritura describe las 8
operaciones; y los documentos de estado dicen la verdad. La CLI queda **propuesta**, no construida.
