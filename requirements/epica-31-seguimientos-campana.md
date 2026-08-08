# E31 — Los dos seguimientos que dejó la campaña de bugfixes

> **Origen**: los dos hallazgos que la campaña de bugfixes (E28 → E29 → E30) destapó **sin poder
> cerrarlos**, porque exigían criterio del usuario y no cabían en ninguna historia de su alcance.
> El commit `60b85ba` los promovió de nota suelta a ficha propia:
> [`decisiones §25`](../decisiones/25-superficie-muerta-revert-transaction.md) (superficie pública
> muerta, origen **mutantes**) y [`decisiones §26`](../decisiones/26-replace-text-noop-reserializa.md)
> (un `replace_text` sin coincidencias reescribe el fichero, origen **juez ciego**).
> [`decisiones/README.md`](../decisiones/README.md) (orden de trabajo, punto 2) las declara «el
> siguiente trabajo natural»: acotadas, con recomendación escrita y criterio de aceptación listo.
>
> **Objetivo de la épica**: cerrar las dos fichas. (1) `Workspace::revert_transaction` deja de ser
> superficie pública, por el mismo mecanismo con que `§16(g)` cerró sus cuatro hermanas en `E29-H10`;
> (2) el camino de escritura deja de reserializar el frontmatter que nadie pidió tocar, y el plan
> gana la señal —hoy inexistente— de «ejecuté tu operación, resultado: sin efecto».
> Referencias maestras: `ARCHITECTURE.md §19.5` (modelo transaccional), `§20.4` (edición de
> frontmatter), `contracts/mcp.yml`, `CLAUDE.md` (invariantes #1, #3 y #5).

**Principio rector**: el invariante #3 del repo —**una sola verdad de patcheo**— aplicado al último
sitio donde no rige. E16-H04 hizo quirúrgica la edición del *frontmatter*; el brazo que reescribe el
*cuerpo* se quedó reconstruyendo el documento entero, y por eso reformatea lo que no se le pidió.
H02 no inventa doctrina: extiende a `ReplaceBody` la que `PatchFrontmatter` ya cumple.

Y su corolario, heredado de E29-H10 y E29-H11: **cuando una superficie pública no tiene consumidor,
el repo la retira, no le escribe un test** — escribirlo consagraría por contrato algo que nadie usa.

## Decisiones de alcance tomadas con el usuario (2026-08-08)

| Punto | Decisión |
|---|---|
| §25 | Salida **(1)**: repliegue a `pub(crate)`. La gemela viva `apply_transaction` **no se toca**. |
| §26 arreglo | Salida **(2)**: preservar los bytes del frontmatter cuando ninguna operación lo toca. |
| §26 reporte | **Ampliación sobre la ficha**: el plan declara explícitamente las operaciones sin efecto. |
| §26 short-circuit | **Descartado**: eliminar la op del plan la borraría de `normalizedOperations` y el agente quedaría peor informado que hoy. |

## Hallazgos de la instrucción que corrigen o amplían las fichas

Verificados leyendo el código y —el último— **ejecutándolo**:

1. **§25 invierte la relación entre las dos funciones.** La ficha dice que `revert_transaction` «es
   el cuerpo que `revert_transaction_con_recibo` envuelve» y con eso desaconseja la retirada total.
   Es al revés: `revert_transaction` (`crates/lodestar-workspace/src/recovery.rs:1083-1091`) es un
   **wrapper de 3 líneas** sobre `revert_transaction_con_recibo` (`:1127`), que es el cuerpo real.
   La salida elegida sigue siendo el repliegue, pero por **reversibilidad**, no porque retirarla
   obligara a reorganizar nada.

2. **§25 tiene un par gemelo.** `apply_transaction` (`transaction.rs:357`) /
   `apply_transaction_con_recibo` (`:406`) son estructuralmente idénticos (wrapper de 3 líneas que
   aplana `PublishedTransaction` a tupla). La diferencia: `apply_transaction` tiene ~21 llamadores en
   tests; `revert_transaction`, **cero**. El patrón es sistemático en forma, con una sola mitad viva.

3. **§26 no se arregla conservando el `ParsedFrontmatter`.** `build_raw_with_bom`
   (`crates/lodestar-core/src/model.rs:364-375`) serializa `fm.value` e **ignora `fm.raw`** siempre,
   venga de `from_mapping` o de `parse_file`. La solución es el **splice**: reconstruir el fichero
   como `raw[..body_offset] + cuerpo`, reutilizando `SplitFront::body_offset` (`model.rs:78-83`), que
   ya existe. Efecto colateral: escribir pasa a ser el **inverso exacto** de leer
   (`SplitFront::body` es `raw[body_offset..]`), en vez de dos funciones que coinciden por cuidado
   manual.

4. **El radio de §26 es mucho mayor que su síntoma.** Todo lo que produce `ReplaceBody` comparte el
   defecto: `plan.rs:533` (`replace_text`), `:581` (`edit_section`), `:638` (`replace_body`), `:1021`
   y `:1045` (**`move`, incluido `rewriteInboundLinks`, que reescribe el cuerpo de CADA enlazante**)
   y `:1100` (`delete remove_links`). Un `move` puede hoy reformatear el frontmatter de medio
   workspace.

5. **Dos defectos más de la misma familia, ninguno con test** — el splice los cierra sin código
   adicional:
   - **Línea en blanco inyectada**: `build_raw_with_bom:373-374` normaliza el separador a `---\n\n`,
     así que un `.md` escrito `---\n…\n---\ncuerpo` vuelve con una línea de más. Hay fixtures reales
     con esa forma (`crates/lodestar-mcp/tests/e2e_migracion.rs:100,105`).
   - **Frontmatter ilegible borrado en silencio (pérdida de datos)**: `parse_file` devuelve
     `frontmatter: None` tanto para «no hay bloque» como para «hay bloque con YAML inválido»
     (`model.rs:711-745`), así que un `ReplaceBody` sobre un documento con frontmatter roto
     **elimina el bloque entero del usuario**. Es exactamente la trampa que el rustdoc de
     `patch_frontmatter` documenta y evita (`model.rs:420-424`): el hermano la esquiva, este brazo
     cae en ella.

6. **El riesgo del lote vacío está DESPEJADO, y se despejó ejecutando.** Con el splice, un no-op deja
   de tocar disco y la transacción publica un lote de **cero paths** — camino que hoy es inalcanzable
   y no tiene ningún guard. Se verificó con un test escrito antes que nada
   (`transaccion_con_lote_vacio_no_degenera`, `crates/lodestar-workspace/tests/transactions.rs`):
   aplicar **y revertir** un lote vacío funcionan, sin error, sin mover la revisión y dejando el `.md`
   byte a byte. **Nota de método**: el primer intento de ese test falló, y falló porque no conseguía
   *producir* un lote vacío — el `ReplaceBody` con el mismo cuerpo cambiaba bytes igualmente. Es §26
   reproducido aislado en la capa de transacción, y confirma el hallazgo 4.

7. **La doc de usuario ya nombra el hueco que (B) cierra.** `docs/user/safe-changes.md:240-241`
   describe el no-op como «no diagnostic, no warning, **no field saying "zero replacements"**».

## Fuera de alcance (explícito)

- **`apply_transaction` y su variante** (hallazgo 2): tiene llamadores, no es superficie muerta.
- **Eliminar la operación no-op del plan**: descartado arriba con motivo.
- **`§14`** (store sin consumidor), **`§9`** (banco de pruebas), **`§20`** (renombrado), **`§21`**
  (comillas), **`§22`**, **`§24`**, **`§16(h)`** y **`§16(k)`**: no las toca ni las reabre.
- **Cambiar el comportamiento de `build_raw`/`build_raw_with_bom`**: siguen existiendo y siguen
  reserializando, porque `Create` (`plan.rs:1252`) y `document_set.rs:251,310` los usan sobre
  documentos **sin bytes previos que preservar**, donde reserializar es lo correcto.

## Historias

| ID | Cierra | Título | Frontera | Fase roja |
|---|---|---|---|---|
| E31-H01 | `§25` | `Workspace::revert_transaction` deja de ser superficie pública | no | no (retirada de superficie) |
| E31-H02 | `§26` | El frontmatter no se reserializa, y el plan declara los no-op | **sí** | sí |

Son **independientes** (ficheros disjuntos: H01 en `lodestar-workspace`, H02 en
`lodestar-core`/`lodestar-app`) y paralelizables. H01 primero por acotada.

---

## E31-H01 — `Workspace::revert_transaction` deja de ser superficie pública

- **Objetivo**: `Workspace::revert_transaction` deja de ser `pub`. Hoy cualquiera fuera del crate
  puede llamarla y obtener una reversión que **no registra recibo durable**, esquivando la garantía
  que E25-H05 introdujo — y es inofensiva solo porque nadie la llama. Misma trampa con fecha de
  caducidad que `E29-H10` cerró para sus cuatro hermanas.

- **Síntoma**: ninguno observable. La evidencia es **medida, no leída**: la pasada de `/mutantes` de
  `§16(l)` sustituyó su cuerpo entero por `unreachable!()` y **los 52 binarios de test del workspace
  siguieron en verde**. `grep` de `.revert_transaction(` sobre el repo entero devuelve **0 llamadas**;
  las ~25 apariciones del identificador son prosa (rustdoc y comentarios). La fachada usa
  `revert_transaction_con_recibo` (`crates/lodestar-app/src/lib.rs:2262`).

- **Referencias**: `decisiones §25` (salida **(1)** elegida) · `E29-H10`
  (`requirements/epica-29-honestidad-superficie.md:1068`, el molde exacto) · `E29-H11` (`§16(b)`,
  retirada completa) · `crates/lodestar-workspace/src/recovery.rs` (`revert_transaction:1083`,
  `revert_transaction_con_recibo:1127`) · `ARCHITECTURE.md §19.5`.

- **Alcance**:
  - `pub fn revert_transaction` → `pub(crate) fn revert_transaction` (`recovery.rs:1083`).
  - Su doc-comment **declara** que no persiste recibo y por eso no se ofrece fuera del crate,
    nombrando `revert_transaction_con_recibo` como la vía pública.
  - **Anclas de rustdoc** (el único riesgo real): `recovery.rs:1030`, `:1093`, `:1122` y
    `failpoints.rs:169-186` la referencian con enlaces intra-doc. Un enlace a un ítem `pub(crate)`
    desde rustdoc público **falla bajo `RUSTDOCFLAGS="-D warnings"`**, que el CI ejecuta. Hay que
    reescribirlos apuntando a la variante pública o desenlazarlos.
  - `contracts/mcp.yml:1199` afirma que `change_revert` «delega en `Workspace::revert_transaction`»:
    es **prosa desactualizada** (delega en la variante con recibo). Se corrige.

- **Fuera de alcance**: `apply_transaction`/`apply_transaction_con_recibo`; retirar la función del
  todo (`§25` eligió replegar, que es lo reversible); cambiar el comportamiento de nada.

- **Criterios de aceptación**:
  - **Dado** el crate tras la historia, **Cuando** se compila el workspace **sin** features de test,
    **Entonces** `Workspace::revert_transaction` **no** forma parte de su API pública → verificación
    estructural: revisión de diff + `cargo doc` (no aparece en la doc pública del crate).
  - **Dado** `lodestar-app`, `lodestar-cli` y `lodestar-mcp`, **Cuando** se compilan, **Entonces**
    compilan **sin cambios** (no la usaban).
  - **Dado** la suite completa (`--workspace` + los dos crates con `--features test-failpoints`),
    **Cuando** se ejecuta, **Entonces** está en verde **y no se ha añadido ningún test que ejerza la
    función**: si hiciera falta uno, la premisa de `§25` era falsa y hay que reabrir el análisis
    (criterio explícito para el juez).
  - **Dado** `cargo doc --workspace --no-deps` con `RUSTDOCFLAGS="-D warnings"`, **Cuando** corre,
    **Entonces** termina sin warnings: ningún enlace intra-doc quedó apuntando a un ítem no público.
  - **Dado** `revert_transaction_con_recibo`, **Cuando** se revisa su visibilidad, **Entonces**
    **sigue siendo `pub`** → control anti-vacuo: la historia no puede llevarse por delante la vía
    legítima por proximidad.

- **Dependencias**: ninguna.

- **Pruebas**: **la evidencia es compilación + suite + revisión de diff**, no un test rojo→verde. No
  hay comportamiento observable que cambie. Se declara explícitamente, en línea con `E29-H10` y con
  el criterio de cierre de `docs/qa/campana-bugfixes-2026-08.md`.

- **Frontera (`mcp.yml`)**: no. La API de `Workspace` no es superficie de wire (solo se corrige la
  prosa de `:1199`).

- **Proceso**: ciclo **acotado**, sin fase roja. Sí **juez ciego**, con el encargo de verificar que no
  se añadió cobertura para justificar la superficie.

---

## E31-H02 — El frontmatter no se reserializa, y el plan declara los no-op

Dos partes en una historia porque comparten camino, tests y documentación: (A) sin (B) dejaría al
agente **peor informado que hoy** —el documento desaparecería de `modified` sin que nada explique la
operación—, y (B) sin (A) documentaría un churn que se puede eliminar.

- **Objetivo**: (A) que el camino de escritura conserve los bytes del frontmatter cuando ninguna
  operación lo toca; (B) que el plan declare explícitamente las operaciones que se materializaron sin
  cambiar nada.

- **Síntoma** (reproducido por el wire sobre `examples/demo/overview.md`, y de nuevo aquí en la capa
  de transacción — hallazgo 6): un `replace_text` cuyo patrón no casa ninguna ocurrencia reescribe el
  fichero. `tags: [atlas, overview]` (estilo *flow*) vuelve como lista en estilo *bloque*;
  `semanticDiff.modified` contiene el documento mientras `bodyChanges` y `frontmatterChanges` van
  **vacíos**. Churn de bytes sin cambio semántico, en un camino invocado esperando que no hiciera
  nada: contamina el `git diff` de quien versione su workspace y hace avanzar `workspaceRevision`
  por una operación vacía.

- **Referencias**: `decisiones §26` (salida **(2)** elegida, más la ampliación de reporte) ·
  `crates/lodestar-core/src/plan.rs` (`normalize_replace_text:512`, `parsed_of:1218`, brazo
  `ReplaceBody:1262`, brazo `PatchFrontmatter:1255`) · `crates/lodestar-core/src/model.rs`
  (`build_raw_with_bom:364`, `SplitFront::body:69`, `body_offset:78`, `splice:511`,
  `patch_frontmatter:425`, `PatchedDocument.reserialized:383`) ·
  `crates/lodestar-app/src/lib.rs` (`PlanResult:3199`, comparación de bytes `:1751-1755`,
  `plan_hash:3066`) · `crates/lodestar-workspace/src/transaction.rs` (`affected_paths:236`, `:434`) ·
  E16-H04 (patch quirúrgico, el precedente), E23-H03 (ausencia de frontmatter), E24-H01 (BOM),
  E29-H07 (`#[serde(default)]` en `PlanResult`).

### Alcance (A) — el splice

- Función nueva en `model.rs`, junto a `splice`:
  `pub fn replace_body_preservando_cabecera(raw: &str, body: &str) -> String`, que devuelve
  `raw[..split_front(raw).body_offset(raw)] + body`. Los tres brazos de `SplitFront` colapsan en uno:
  - `Bloque` → prefijo = BOM + bloque + delimitadores + separador, literal;
  - `Sin` → prefijo = BOM (E24-H01) y nada más (E23-H03: la ausencia se conserva);
  - `SinCerrar` → igual que `Sin`, que es justo lo que `SplitFront::body` lee, así que la simetría
    lectura/escritura es exacta.
- El brazo `ReplaceBody` de `apply_one` (`plan.rs:1262-1279`) pasa a llamarla.
- `parsed_of` (`plan.rs:1218-1227`) se queda **sin llamadores y se borra**; su rustdoc (que documenta
  E23-H03 y E24-H01) **se migra** a la función nueva: las lecciones siguen vivas, solo que ahora las
  garantiza la estructura en vez de la coordinación de dos piezas.
- Se corrige el rustdoc de `build_raw_with_bom` (`model.rs:349-350`), que afirma que el brazo
  `ReplaceBody` lo usa — dejará de ser cierto.

### Alcance (B) — la señal de no-op

- Tipo nuevo en `lodestar-app`: `NoOpOperation { index: usize, path: RelPath, op: String }`, con `op`
  en el vocabulario snake_case del wire (`plan::op_variant_name`).
- Campo nuevo **solo en `PlanResult`** (`lib.rs:3199`): `no_op_operations: Vec<NoOpOperation>` con
  `#[serde(default)]`, junto a `policy` y `captured_revisions` —el precedente exacto de «proyección
  que acompaña al plan sin entrar en su identidad»—.
- **Fuera del `planHash` por construcción**: el hash cubre `baseWorkspaceRevision ‖
  normalizedOperations` (`lib.rs:3066-3073`), así que **ningún `planHash` existente cambia** y los
  planes ya persistidos siguen aplicándose. Es además lo semánticamente correcto: el hash es la
  identidad de *lo que se pidió*; «resultó no-op» es una propiedad *del resultado*.
- **Derivado, no declarado**: se computa con la comparación de bytes que `change_plan` **ya hace**
  (`lib.rs:1751-1755`), el mismo predicado que el escritor usa en `affected_paths`
  (`transaction.rs:236-249`). Cero criterio nuevo, cero riesgo de divergir del disco.
- **El predicado cubre cualquier op sin efecto**, no solo `replace_text`: también un `edit_section`
  que reescribe una sección con su contenido actual, un `patch_frontmatter` que escribe el valor que
  ya estaba y un `move` con `from == to`.
- **La operación NO se elimina de `normalizedOperations`**: la señal es aditiva (requisito explícito).
- El rustdoc de `PatchedDocument.reserialized` (`model.rs:383-385`) afirma que «lo consume
  `change_plan` (E21) para declararlo en el plan», y **ese consumo no existe**. No sirve como vehículo
  de (B) —responde a otra pregunta—, pero **deja de mentir** en esta historia.

- **Fuera de alcance**: eliminar la op del plan; cambiar `build_raw`/`build_raw_with_bom`; tocar
  `crates/lodestar-mcp/src/tools.rs` (el `outputSchema` deriva de `schema_for!(PlanResult)`, se
  actualiza solo); el `inputSchema` de `change_plan`.

- **Criterios de aceptación**:
  - **Dado** un documento con frontmatter en estilo *flow*, **Cuando** un `replace_text` no casa
    ninguna ocurrencia, **Entonces** el fichero en disco queda **byte a byte idéntico** (mtime
    aparte), **no** aparece en `semanticDiff.modified` y `workspaceRevision` **no** avanza.
  - **Dado** un documento con frontmatter *flow*, **Cuando** se le cambia **solo el cuerpo**,
    **Entonces** conserva el estilo *flow* de su frontmatter, con sus comillas y comentarios YAML.
  - **Dado** un documento escrito `---\n…\n---\ncuerpo` (sin línea en blanco), **Cuando** se le
    reescribe el cuerpo con el mismo contenido, **Entonces** los bytes son idénticos: no se inyecta
    separador (hallazgo 5).
  - **Dado** un documento con frontmatter **ilegible** (bloque cerrado con YAML inválido, y su gemelo
    sin cerrar), **Cuando** una operación reescribe su cuerpo, **Entonces** el bloque **sobrevive
    literal** — hoy se borra, y es pérdida de datos (hallazgo 5).
  - **Dado** un `move --rewriteInboundLinks` sobre un documento enlazado desde otros con frontmatter
    *flow*, **Entonces** los enlazantes conservan el estilo de su frontmatter (el radio ampliado).
  - **Dado** un documento **sin** bloque de frontmatter y otro **con BOM**, **Cuando** se les
    reescribe el cuerpo, **Entonces** siguen sin bloque / conservando el BOM (anti-regresión E23-H03
    y E24-H01).
  - **Dado** un `replace_text` sin coincidencias, **Cuando** se planifica, **Entonces**
    `noOpOperations` lo declara con su `path` y su `op`, **y** la operación **sigue apareciendo** en
    `normalizedOperations`.
  - **Dado** un plan con una operación efectiva y otra vacía, **Entonces** `noOpOperations` lista
    **solo** la vacía → anti-vacuo: la señal discrimina, no lista todo.
  - **Dado** un plan persistido por un binario anterior (sin la clave `noOpOperations`), **Cuando** se
    carga y se aplica, **Entonces** funciona con la lista vacía por defecto.
  - **Dado** una transacción cuyo lote afectado es **vacío**, **Cuando** se aplica y luego se
    revierte, **Entonces** ninguna de las dos falla, `changedPaths` va vacío y el `.md` queda byte a
    byte → **ya verificado** por `transaccion_con_lote_vacio_no_degenera`.

- **Dependencias**: ninguna respecto a H01.

- **Pruebas** (fase roja real; el autor de tests las escribe **antes** y deben fallar):
  - **Core** (`crates/lodestar-core/tests/documento.rs`): frontmatter *flow* con comentario y comillas
    preservado byte a byte al cambiar el cuerpo (con anti-vacuo: el cuerpo **sí** cambió); separador
    sin línea en blanco; frontmatter ilegible (dos variantes); la familia entera (`edit_section`,
    `move --rewriteInboundLinks`, `delete remove_links`); `workspace_revision` quieta.
  - **App** (`crates/lodestar-app/tests/plan.rs`, `escritura.rs`): e2e `change_plan` + `change_apply`
    con bytes idénticos en disco y `changedPaths` vacío; `noOpOperations` con op efectiva mezclada;
    plan persistido sin la clave (reusando `forja_plan_persistido`, `plan.rs:689-701`).
  - **Test a INVERTIR**: `replace_text_sin_ocurrencias_en_forma_array_es_noop`
    (`crates/lodestar-app/tests/plan.rs:1847-1913`). Sus aserciones `:1891-1898` exigen hoy que
    `flow.md` **sí** esté en `modified`; el propio mensaje dice qué hacer cuando esto se arregle
    («invierte la aserción y actualiza el caveat de `docs/user/safe-changes.md`»). La `:1899-1905`
    (`block.md`) se conserva y cambia de papel. Actualizar el bloque de comentario `:1799-1815`.
    > **Ojo**: tras (A) los dos documentos del fixture se comportan igual, así que **el fixture deja
    > de ser anti-vacuo por sí solo** y un `apply_one` que no escribiera nunca nada pasaría el test.
    > Hace falta una aserción nueva: un `replace_text` que **sí** casa sobre un tercer documento en
    > *flow* debe aparecer en `modified`/`bodyChanges` **y** conservar su `tags: [a, b]` literal.
  - **Test que hoy pasa por la razón equivocada**: `bom_roundtrip_byte_a_byte`
    (`documento.rs:2754`) — su fixture `DOC_CON_BOM` (`:2521-2531`) ya está en forma canónica *block*,
    así que reserializarlo devuelve los mismos bytes **de casualidad**. Añadirle una clave en *flow*.
    Es el mismo defecto de método que originó `§26` (ver su «Nota de método»).

- **Frontera (`mcp.yml`)**: **sí**.
  ```yaml
  # change_plan · retorno (~:1069) — AÑADIR al PlanResult:
  noOpOperations: "[{ index, path, op }] — operaciones que se materializaron pero cuyo resultado es
    IDÉNTICO al documento de partida (§26, E31-H02). Ni error ni advertencia: la respuesta honesta a
    «ejecuté tu operación, resultado: sin efecto», que hasta v0.5.0 el plan no sabía dar (la op
    aparecía en normalizedOperations indistinguible de una efectiva). DERIVADO por comparación de
    bytes, el mismo predicado con que el escritor computa su lote afectado. Va FUERA del planHash —
    que cubre baseWorkspaceRevision ‖ normalizedOperations—, así que ningún planHash cambia y un plan
    persistido por un binario anterior se lee con la lista vacía. La op NO se elimina de
    normalizedOperations: la señal es aditiva."
  ```
  Más una frase en `semantica:` y la entrada en la cabecera de cambios. `/contrato --check` al final.

- **Docs**: `docs/user/safe-changes.md` — **retirar** el caveat `:256-262` (lo resuelve (A)) y
  reescribir `:238-252`, cuyo texto dice «no field saying "zero replacements"», que (B) deja de ser
  cierto (y el no-op deja de ser *silent*). `ARCHITECTURE.md:1125` (el plan declara si el bloque se
  reserializará): anotar que `ReplaceBody` ya no reserializa nunca.

- **Proceso**: `/tdd` completo (autor-tests ≠ implementador; el implementador **no** puede tocar los
  tests), `/contrato --check`, `/mutantes` sobre `model.rs` y `plan.rs` —la función nueva es de una
  línea y `cargo-mutants` podría no distinguir sus brazos; los tests de separador y de frontmatter
  ilegible son los que matan las mutaciones interesantes— y **juez ciego** con encargo específico:
  **verificar ejecutando el binario** que los bytes en disco no cambian. La lección de E23 y E30 es
  que leer el código no bastó ni una sola vez.

## Riesgos declarados

- **Lote vacío** — ✅ **despejado ejecutando** (hallazgo 6), antes de escribir una línea de
  implementación.
- **Planes persistidos entre binarios**: un plan calculado por el binario viejo y aplicado por el
  nuevo pasa el gate de `planHash` (las ops son las mismas) pero produce bytes distintos, así que su
  `semanticDiff` congelado no coincide con lo escrito. El desajuste es siempre en la dirección segura
  (se escribe **menos** de lo prometido) y lo acotan el TTL corto del plan y la re-verificación del
  hash. Se documenta en el contrato; no lleva código.
- **Hash / store / paridad**: sin riesgo, y **mejora**. `workspace_revision` (`types.rs:1295-1311`),
  `DocumentRevision::from_hash` y el gate del store hashean bytes crudos, así que preservar bytes
  **reduce** el movimiento espurio de revisión — el mismo argumento con que E24-H01 justificó
  conservar el BOM. Hoy el mismo documento tiene dos bytes posibles según haya pasado o no por un
  `ReplaceBody`; eso es lo que se elimina.

## Cierre de la épica

- `decisiones/25-*.md` y `decisiones/26-*.md` → `estado: cerrada`, con `cerrada_en` y `revisada_en`.
- Filas de `§25` y `§26` en `decisiones/README.md`, y el punto 2 de su orden de trabajo (que las
  señala como «el siguiente trabajo natural»).
- `IMPLEMENTATION_STATUS.md`: épica E31 y qué invariantes quedan verificados.
- `docs/qa/campana-bugfixes-2026-08.md`: los dos seguimientos, cerrados.
- `CHANGELOG.md`: entrada de fix (el churn de bytes y la pérdida de datos del frontmatter ilegible
  son observables por el usuario).
