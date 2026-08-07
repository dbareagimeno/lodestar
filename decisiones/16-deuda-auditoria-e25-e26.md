---
id: 16
titulo: "Deuda declarada por la auditoría de E25 y E26"
estado: "cerrada"
prioridad: 4
etiquetas: ["escritura", "mcp", "store", "lenguaje-consulta", "docs"]
origen: "juez-ciego"
abierta_en: "2026-07-29"
cerrada_en: "2026-08-02"
revisada_en: "2026-08-02"
epica: "E25"
subpuntos: ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"]
relacionadas: [3, 14, 15, 21]
---

# §16 — Deuda declarada por la auditoría de E25/E26 — **DISUELTA**

> **Cerrada por disolución el 2026-08-02**, en la repriorización conjunta de las decisiones. Doce
> deudas de naturalezas incompatibles bajo una sola prioridad no se pueden priorizar: una ficha con
> prioridad 4 describía mal tanto a (e) —una salvaguarda de escritura que no existe sin avisar—
> como a (h) —una ventana estrecha sobre un fichero auxiliar—. Cada punto pasa a su **dueño real**;
> el registro de origen se conserva **aquí** para que la próxima auditoría no lo redescubra como
> hallazgo nuevo, que es lo que E23 y E24 pagaron dos veces.
>
> Ninguno bloqueó el merge de E25/E26, y ninguno era un defecto observable el día que se registró.

## Mapa de destinos

| Punto | Qué era | Destino (2026-08-02) |
|---|---|---|
| (a) | *Quoting* en el lenguaje de consulta | → ficha propia [`§21`](21-comillas-lenguaje-consulta.md). **Decidido: añadir comillas**, con puerta de diseño. |
| (b) | `Envelope` sin llamantes | **Decidido: retirarlo.** Trabajo, no decisión. **Saldado por `E29-H11`** (commit `7f519d2`): `Envelope<T>`/`ErrorEnvelope` retirados de `lodestar-app`. |
| (c) | Cache y watcher sin uso en producción | → absorbido por [`§14`](14-store-sin-consumidor.md). |
| (d) | MCP monohilo, sin *timeout* ni cancelación | → absorbido por [`§3`](03-transporte-mcp-rmcp.md). |
| (e) | La config no rechaza claves desconocidas | **Decidido: estricto.** Historia propia y primera de la épica de honestidad. **Saldado por `E29-H01`** (commit `4a52f59`): `deny_unknown_fields` + familias de `validation` contra lista cerrada. |
| (f) | Workspace vacío indistinguible de directorio equivocado | **Decidido: aviso `warn`, sin tocar exit codes.** **Saldado por `E29-H06`** (commit `88e99b2`): diagnóstico `WORKSPACE-EMPTY` (warn). |
| (g) | API pública de `Workspace` no transaccional | **Decidido: cerrarlas al exterior** (`pub(crate)` / solo test). **Saldado por `E29-H10`** (commit `7f519d2`): `create_document`/`write_document`/`merge_frontmatter` replegadas. |
| (h) | Escritores de runtime sin lock | **Registrado sin acción**; se cierra si aparece un caso real o al tocar el GC. Único punto que sigue vivo aquí. |
| (i) | Secuencia de sellado duplicada `apply`/`revert` | **Saldado por `E28-H01`** (commit `296147b`): extraída a `seal_published_transaction`, único camino compartido. |
| (j) | Un cursor basura reinicia la paginación en silencio | **Decidido: `INVALID_SCHEMA`**, en el ciclo de higiene. |
| (k) | Matriz de trazabilidad sin filas de E15–E24 | **Decidido: historia propia**, con cada fila verificada contra su épica. |
| (l) | Deuda de fuerza de suite (mutantes) | **Decidido: pasada de `/mutantes` acotada** en el ciclo de higiene. La divergencia core↔store de E26-H09 va con [`§14`](14-store-sin-consumidor.md). |

---

## Registro de origen (se conserva íntegro)

### (a) *Quoting* en el lenguaje de consulta: tres límites latentes

- **Origen**: **E26-H09** (un solo dialecto de dot-paths). Al unificar `metadata_inspect` con
  `build_field_path` quedaron a la vista tres casos que el dialecto único **no puede expresar**:
  una clave de frontmatter con **punto literal** (`sonar.projectKey`); una clave llamada literalmente
  **`frontmatter`**, tapada por el anclaje de E24-H08; y la **fusión de nombres** entre `a.b` literal
  y `a` → `b` anidado.
- **Por qué no se cerró en E25/E26**: los tres piden **sintaxis nueva**, o sea abrir `§20.8`, que es
  una decisión de diseño con puerta propia. Y hoy no son silenciosos.
- **Destino**: [`§21`](21-comillas-lenguaje-consulta.md), donde además se corrige la imprecisión del
  caso (3) que detectó [`§19(c)`](19-hallazgos-referencia-usuario.md).

### (b) `Envelope`/`ErrorEnvelope` no tienen llamantes

- **Origen**: auditoría de la superficie (UX de errores).
- **Qué es**: el envelope de `lodestar-app` (E10-H01, decisión **D3** de `§0`) existe, compila y está
  testeado, pero **ninguna fachada lo usa**: MCP devuelve `structuredContent` + texto con el código, y
  la CLI sus exit codes. Es capacidad construida sin consumidor, como el store de
  [`§14`](14-store-sin-consumidor.md).
- **Por qué no se cerró en E25/E26**: E26 trabajó sobre la superficie **real**. Meter el envelope
  habría sido cambiar la forma del wire en la misma tanda que arreglaba su contenido.
- **Decidido (2026-08-02): retirarlo**, como se retiró `lodestar-vcs` en E15-H01. Tras E26-H07 el
  wire ya es honesto sin envelope; mantener dos formas de respuesta —una en uso y otra por si
  acaso— es la duplicación que el invariante #4 existe para evitar.
- **Saldado (2026-08-07) por `E29-H11`** (commit `7f519d2`, épica
  [`epica-29-honestidad-superficie.md`](../requirements/epica-29-honestidad-superficie.md)):
  `Envelope<T>`/`ErrorEnvelope` retirados de `lodestar-app`; ninguna fachada perdió capacidad porque
  ninguna los consumía.

### (c) La cache SQLite y el watcher siguen sin uso en producción

- **Origen**: auditoría de la superficie; es la **misma deuda de [`§14`](14-store-sin-consumidor.md)**,
  vista desde el otro lado.
- **Qué añade**: no solo el store no tiene consumidor — el **watcher** (E3-H04, el «único escritor
  reconcilia» del invariante #5) tampoco corre en el motor headless: sin `enable_cache` no hay nada
  que reconciliar. El invariante #5 se sostiene hoy por el **protocolo de escritura** (temp+fsync+
  rename por el único camino), no por el watcher.
- **Destino**: [`§14`](14-store-sin-consumidor.md), en la misma decisión.

### (d) Servidor MCP monohilo, sin *timeout* ni cancelación

- **Origen**: auditoría de la superficie.
- **Qué es**: el bucle JSON-RPC atiende **una petición a la vez** y no hay forma de cancelar ni de
  acotar en el tiempo una llamada larga (`knowledge_check` sobre una base grande, un `change_plan`
  con selección masiva). Un cliente que se impaciente no tiene protocolo para decirlo.
- **Destino**: [`§3`](03-transporte-mcp-rmcp.md). Escribir cancelación a mano sobre el stdio propio
  para luego migrar a `rmcp` sería trabajo tirado.

### (e) La config no rechaza claves desconocidas, y una config ilegible cae a *defaults* en silencio

- **Origen**: auditoría del camino de escritura.
- **Qué es**: `WorkspaceConfig` no lleva `#[serde(deny_unknown_fields)]`, así que un
  `writableRoots` mal escrito (`writable_roots`, `writeableRoots`) se **ignora sin avisar** y el
  workspace queda con la política por defecto — es decir, **más permisivo** que lo que el usuario
  cree. Y un `.lodestar/config.yaml` ilegible degrada a *defaults* en silencio, cuando la CLI ya fija
  el precedente contrario: config inválida era exit 3, no *defaults*.
- **Decidido (2026-08-02): estricto**, en la dirección que ya recomendaba la ficha. Es una
  salvaguarda de escritura que el usuario cree haber puesto y no está, y nadie se lo dice: forma de
  fallo de seguridad, no tolerancia amable. **Historia propia y primera de la épica de honestidad**,
  separada de [`§15`](15-parametros-no-declarados.md) para que su cierre no dependa del trabajo
  mayor del wire. Momento más barato posible: sin usuarios externos, la ruptura es nula.
- **Saldado (2026-08-07) por `E29-H01`** (commit `4a52f59`, épica
  [`epica-29-honestidad-superficie.md`](../requirements/epica-29-honestidad-superficie.md)):
  `deny_unknown_fields` en `WorkspaceConfig` y todas sus secciones, más las claves de `validation`
  contra la lista cerrada `VALIDATION_FAMILIES`; config ilegible distingue de config ausente (exit 3
  en ambas fachadas, no *defaults* silenciosos).

### (f) Un workspace vacío es indistinguible de un directorio equivocado

- **Origen**: auditoría de la superficie.
- **Qué es**: `cd` a un directorio que no es el que se creía (o donde la `DiscoveryPolicy` excluye
  todo) da `workspace_status` con 0 documentos y `lodestar check` **exit 0 · VÁLIDO**. La respuesta es
  literalmente correcta —no hay nada mal— y prácticamente engañosa: es el «respondió que sí a algo que
  no entendió» del principio rector de E26, en la puerta de entrada.
- **Decidido (2026-08-02): (a) avisar sin cambiar el exit code** — diagnóstico de nivel `warn` «0
  documentos descubiertos bajo esta raíz». Conserva el contrato de la puerta de CI (un repo
  legítimamente vacío sigue pasando) y cierra el engaño en la primera experiencia de cualquier
  usuario nuevo. Entra en la épica de honestidad.
- **Saldado (2026-08-07) por `E29-H06`** (commit `88e99b2`, remate `6a3a6ca`, épica
  [`epica-29-honestidad-superficie.md`](../requirements/epica-29-honestidad-superficie.md)):
  diagnóstico `WORKSPACE-EMPTY` (warn) cuando la raíz no descubre ningún documento; exit codes
  intactos.

### (g) API pública de `Workspace` no transaccional (defecto **S8** de la auditoría)

- **Origen**: auditoría del camino de escritura (S8), confirmado por los jueces de E25.
- **Qué es**: `create_document`, `write_document`, `merge_frontmatter` y `publish` son **públicos** y
  escriben el canónico **sin lock, sin journal y sin copias de recuperación** — o sea, esquivan las
  seis garantías que E25 reforzó. Hoy son inofensivos porque **no tienen llamadores de producción**
  (solo tests), exactamente el mismo caso que `materialize_staging` en E23-H12.
- **Decidido (2026-08-02): (a) replegar a `pub(crate)`** / marcar como primitivas de test. Una API
  pública que rompe el invariante nuclear del crate es una trampa con fecha de caducidad: funciona
  hasta que alguien la llama. Sin llamadores, cerrarla no cuesta nada. Entra en la épica de honestidad.
- **Saldado (2026-08-07) por `E29-H10`** (commit `7f519d2`, épica
  [`epica-29-honestidad-superficie.md`](../requirements/epica-29-honestidad-superficie.md)):
  `create_document`, `write_document` y `merge_frontmatter` replegadas a `pub(crate)`/primitivas de
  test; ningún llamador de producción perdió capacidad.

### (h) Los escritores de runtime no toman el lock

- **Origen**: **reserva del juez ciego de E25-H03**.
- **Qué es**: `persist_plan` y `write_receipt` escriben bajo `.lodestar/runtime/` **sin** el lock de
  publicación, mientras el barrido de temporales del GC (E24-H06) puede correr **desde otro proceso**.
  La ventana es estrecha y el daño acotado (un plan o un recibo que hay que reescribir, no un `.md`),
  y por eso E25-H03 se limitó a proteger el plano de **recuperación**.
- **Vigente (2026-08-02)**: **registrado sin acción**, prioridad baja. Se cierra si aparece un caso
  real o cuando se toque el GC por otro motivo. Es el único punto de §16 que no tiene destino.

### (i) La secuencia de sellado está duplicada entre `apply` y `revert`

- **Origen**: **reserva del juez ciego de E25-H05**.
- **Qué es**: tras E25-H04/H05, publicar y revertir comparten la **misma coreografía** —promover el
  recibo pendiente, limpiar staging, borrar el journal, fsync del directorio— escrita **dos veces**.
  No es duplicación de *lógica de dominio*, pero sí de **secuencia**, que es donde un arreglo futuro
  se aplicará a una mitad y no a la otra.
- **Decidido (2026-08-02)**: extraer `sellar_publicado(txn_id, journal_path)` compartido, **en un
  ciclo de higiene propio** con la suite actual como red y sin cambio de comportamiento.
- **Saldado (2026-08-06) por `E28-H01`** (commit `296147b`, épica
  [`epica-28-defectos-destructivos-testbench.md`](../requirements/epica-28-defectos-destructivos-testbench.md)):
  la coreografía se extrajo a `seal_published_transaction`, consumida por
  `apply_transaction_con_recibo` y `revert_transaction_con_recibo` por igual. No se ejecutó como
  ciclo de higiene aparte, sino junto al arreglo de M-01 (`decisiones §23`), por compartir la misma
  zona de código — la agrupación que el punto 0 del orden de trabajo ya anticipaba.

### (j) Un cursor basura reinicia la paginación en silencio

- **Origen**: **reserva del juez ciego de E26-H10**.
- **Qué es**: `decode_cursor` interpreta un cursor ilegible como **offset 0**, así que un cursor
  corrupto o de otra tool devuelve **la primera página** en vez de un error. Un agente que pagina en
  bucle con un cursor mal propagado no termina nunca y no se entera.
- **Decidido (2026-08-02)**: `INVALID_SCHEMA` con mensaje, en el **ciclo de higiene** junto a (i) y
  (l). Es barato y coherente con el principio rector de E26.

### (k) La matriz de trazabilidad no tiene filas de E15–E24

- **Origen**: **observación del cierre de E24-H18**, verificada al cerrar E25/E26.
- **Qué es**: `requirements/trazabilidad.md` se quedó en el giro headless (E9–E14). E15–E22, E23 y
  E24 **nunca se trazaron**, pese a que el alcance de E24-H18 lo declaraba. Diez épicas sin fila.
  E25/E26 sí están trazadas, con lo que el hueco queda en medio y a la vista. (E27 tampoco.)
- **Decidido (2026-08-02)**: **historia propia**, con el criterio de que **cada fila se verifique
  contra la épica**, no contra el recuerdo — a la carrera produciría filas plausibles en vez de
  verificadas, que es el defecto que el documento existe para impedir. Trabajo acotado y delegable.

### (l) Deuda de fuerza de suite y flecos menores registrados por los jueces ciegos

- **Origen**: las **reservas MENORES** de los veredictos de E25/E26. Casi todas salieron de *mutation
  testing*, no de un fallo observado: **la suite no muerde ahí**.
- **Qué es**, por historia y con el mutante que lo destapó:
  - **E25-H01** — mutación **(g)**: `paths_divergentes` y el mensaje del conflicto de ventana pueden
    **vaciarse** sin que ningún test muerda. El aborto sigue cubierto; el diagnóstico que dice **qué**
    divergió no lo fija nadie.
  - **E25-H02** — mutación **S**: el sidecar de huellas movido a cuarentena se puede **borrar** sin
    que falle un test, pese a que la cuarentena existe para no perder material forense. Mutación
    **N**: la **numeración** de cuarentenas repetidas (`.2`, `.3`) tampoco está fijada.
  - **E25-H03** — mutación **c**: el **no-op silencioso** del GC dentro de `recover_if_pending` no
    tiene arnés. Está **mitigado** por el testigo tipado (`&WorkspaceLock`), pero mitigado no es
    cubierto.
  - **E25-H04** — mutante **k**: el guard `recibo_a_salvo` no tiene test que **inyecte un fallo de
    promoción**. Tiene **espejo** en el sellado del revert de H05.
  - **E25-H05** — `Workspace::revert_transaction` quedó **sin llamador ni test** propio; y la
    re-verificación es **única** en el revert mientras el apply comprueba **dos veces**.
  - **E26-H09** — **divergencia latente core↔store**: el catálogo publica nombres **anclados**
    (`frontmatter.graph.backlinks`) mientras el store indexa `metadata.field_path` con los nombres
    crudos de `walk`. Hoy nadie lee esa columna.
  - **E26-H10** — la **aritmética de paginación está en 4 copias** y los límites se aplican en **3
    sitios por tool**. La duplicación sigue ahí, misma forma que (i).
- **Decidido (2026-08-02)**: **pasada de `/mutantes` acotada** a los ficheros de E25/E26, con
  presupuesto cerrado, que convierta en test los supervivientes que describan comportamiento real
  —(g), S, N, k son los candidatos claros—, dentro del **ciclo de higiene**. El helper único de
  paginación va con (j) en ese mismo ciclo. La divergencia core↔store de E26-H09 **no se toca aquí**:
  va con [`§14`](14-store-sin-consumidor.md).
