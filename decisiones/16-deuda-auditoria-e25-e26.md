---
id: 16
titulo: "Deuda declarada por la auditoría de E25 y E26"
estado: "abierta"
prioridad: 4
etiquetas: ["escritura", "mcp", "store", "lenguaje-consulta", "docs"]
origen: "juez-ciego"
abierta_en: "2026-07-29"
revisada_en: "2026-07-29"
epica: "E25"
subpuntos: ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"]
relacionadas: [3, 14, 15]
---

# §16 — Deuda declarada por la auditoría de E25/E26

> **Qué es esta sección**: lo que la auditoría del camino de escritura y de la superficie de errores
> (2026-07-29) y los **jueces ciegos de las 11 historias** dejaron **explícitamente fuera** de
> E25/E26. No es una lista de bugs pendientes: es lo que se decidió **no** arreglar ahí, con el
> motivo. Cada punto lleva su **origen**, para que la próxima auditoría no lo redescubra como
> hallazgo nuevo — que es exactamente lo que E23 y E24 pagaron dos veces.
>
> **Ninguno bloquea el merge de E25/E26.** Varios (b, c, g) son *capacidad construida sin consumidor*,
> la misma familia que [`§14`](14-store-sin-consumidor.md), otros (a, j) son límites de sintaxis que hoy son **ruidosos, no
> silenciosos**, que es la propiedad que estas épicas vinieron a garantizar.

### (a) *Quoting* en el lenguaje de consulta: tres límites latentes

- **Origen**: **E26-H09** (un solo dialecto de dot-paths). Al unificar `metadata_inspect` con
  `build_field_path` quedaron a la vista tres casos que el dialecto único **no puede expresar**:
  1. una clave de frontmatter que **contiene un punto literal** (`sonar.projectKey`) — direccionable
     con `FieldPath::from_segments` desde Rust, pero no desde la sintaxis textual, que siempre parte
     por puntos;
  2. una clave del usuario llamada literalmente **`frontmatter`** — el prefijo se interpreta como
     anclaje (E24-H08), así que la clave homónima queda tapada;
  3. la **fusión de nombres**: `a.b` como clave literal y `a` → `b` anidado producen el mismo
     `FieldPath`, así que el catálogo no los distingue.
- **Por qué no se cerró**: los tres piden **sintaxis nueva** (comillas o *escaping* en el lenguaje),
  o sea abrir `§20.8`, que es una decisión de diseño con puerta propia — no un apéndice de una épica
  de endurecimiento. Y hoy **no son silenciosos**: (1) y (3) son casos raros y documentados, y (2)
  produce un resultado explicable, no una respuesta equivocada disfrazada de correcta.
- **Qué decidir**: si el lenguaje gana *quoting* (`frontmatter."sonar.projectKey"`) o si estos tres
  casos se declaran **fuera de alcance por escrito** en `§20.8`.
- **Recomendación**: declararlos por escrito ahora y abrir el *quoting* solo si aparece un caso real.
  Sintaxis nueva sin demanda es superficie que hay que mantener para siempre.

### (b) `Envelope`/`ErrorEnvelope` no tienen llamantes

- **Origen**: auditoría de la superficie (UX de errores).
- **Qué es**: el envelope de `lodestar-app` (E10-H01, decisión **D3** de `§0`) existe, compila y está
  testeado, pero **ninguna fachada lo usa**: MCP devuelve `structuredContent` + texto con el código, y
  la CLI sus exit codes. Es capacidad construida sin consumidor, como el store de [`§14`](14-store-sin-consumidor.md).
- **Por qué no se cerró en E25/E26**: E26 trabajó sobre la superficie **real** (código + mensaje en
  las 10 tools). Meter el envelope habría sido cambiar la forma del wire en la misma tanda que
  arreglaba su contenido.
- **Qué decidir**: (a) **cablearlo** como forma única de respuesta de las dos fachadas; (b)
  **retirarlo** como se retiró `lodestar-vcs`; (c) **acotarlo** por escrito a consumidores futuros.
- **Recomendación**: **(b) o (c)**. Tras E26-H07 el wire ya es honesto sin envelope; mantener dos
  formas de respuesta es la clase de duplicación que el invariante #4 existe para evitar.

### (c) La cache SQLite y el watcher siguen sin uso en producción

- **Origen**: auditoría de la superficie; es la **misma deuda de [`§14`](14-store-sin-consumidor.md)**, vista desde el otro lado.
- **Qué añade a [`§14`](14-store-sin-consumidor.md)**: no solo el store no tiene consumidor — el **watcher** (E3-H04, el «único
  escritor reconcilia» del invariante #5) tampoco corre en el motor headless: sin `enable_cache` no
  hay nada que reconciliar. El invariante #5 se sostiene hoy por el **protocolo de escritura**
  (temp+fsync+rename por el único camino), no por el watcher.
- **Recomendación**: tratarlo **con [`§14`](14-store-sin-consumidor.md)**, en la misma decisión. No merece épica propia separada.

### (d) Servidor MCP monohilo, sin *timeout* ni cancelación

- **Origen**: auditoría de la superficie.
- **Qué es**: el bucle JSON-RPC atiende **una petición a la vez** y no hay forma de cancelar ni de
  acotar en el tiempo una llamada larga (`knowledge_check` sobre una base grande, un `change_plan`
  con selección masiva). Un cliente que se impaciente no tiene protocolo para decirlo.
- **Por qué no se cerró**: es **diseño de transporte**, y su decisión natural va con [`§3`](03-transporte-mcp-rmcp.md) (rmcp
  oficial), que ya contempla el problema desde fuera.
- **Recomendación**: resolverlo **dentro de [`§3`](03-transporte-mcp-rmcp.md)**. Escribir cancelación a mano sobre el stdio propio
  para luego migrar a rmcp sería trabajo tirado.

### (e) La config no rechaza claves desconocidas, y una config ilegible cae a *defaults* en silencio

- **Origen**: auditoría del camino de escritura.
- **Qué es**: `WorkspaceConfig` no lleva `#[serde(deny_unknown_fields)]`, así que un
  `writableRoots` mal escrito (`writable_roots`, `writeableRoots`) se **ignora sin avisar** y el
  workspace queda con la política por defecto — es decir, **más permisivo** de lo que el usuario
  cree. Y un `.lodestar/config.yaml` ilegible degrada a *defaults* en silencio, cuando la CLI ya fija
  el precedente contrario: un `lodestar.toml` inválido era exit 3, no *defaults* (revisión 2026-07).
- **Por qué no se cerró**: es la **misma pregunta que [`§15`](15-parametros-no-declarados.md)** —¿rechazar lo que no se declara?— en el
  fichero de config en vez de en el wire, y merece la misma respuesta para no dejar el repo con dos
  criterios opuestos.
- **Recomendación**: **decidirlo junto con [`§15`](15-parametros-no-declarados.md)**, y en la dirección estricta: una raíz de escritura
  que el motor no aplica porque el usuario escribió mal la clave es un fallo de seguridad silencioso,
  no una tolerancia amable.

### (f) Un workspace vacío es indistinguible de un directorio equivocado

- **Origen**: auditoría de la superficie.
- **Qué es**: `cd` a un directorio que no es el que se creía (o donde la `DiscoveryPolicy` excluye
  todo) da `workspace_status` con 0 documentos y `lodestar check` **exit 0 · VÁLIDO**. La respuesta es
  literalmente correcta —no hay nada mal— y prácticamente engañosa: es el «respondió que sí a algo que
  no entendió» del principio rector de E26, en la puerta de entrada.
- **Por qué no se cerró**: cambiar el veredicto de `check` sobre un workspace vacío es un **cambio de
  contrato de la puerta de CI** (un repo legítimamente vacío pasaría a fallar), y eso pide decisión,
  no arreglo.
- **Qué decidir**: (a) **avisar** sin cambiar el exit code (un diagnóstico de nivel `warn` «0
  documentos descubiertos bajo esta raíz»); (b) **exit distinto**; (c) dejarlo como está.
- **Recomendación**: **(a)**. Conserva el contrato de exit codes y cierra el engaño.

### (g) API pública de `Workspace` no transaccional (defecto **S8** de la auditoría)

- **Origen**: auditoría del camino de escritura (S8), confirmado por los jueces de E25.
- **Qué es**: `create_document`, `write_document`, `merge_frontmatter` y `publish` son **públicos** y
  escriben el canónico **sin lock, sin journal y sin copias de recuperación** — o sea, esquivan las
  seis garantías que E25 acaba de reforzar. Hoy son inofensivos porque **no tienen llamadores de
  producción** (solo tests), exactamente el mismo caso que `materialize_staging` en E23-H12.
- **Por qué no se cerró en E25**: retirarlos o replegarlos a `pub(crate)` toca la API pública del
  crate y la suite que los usa; hacerlo dentro de una épica de endurecimiento habría mezclado un
  cambio de superficie con seis arreglos de concurrencia.
- **Qué decidir**: (a) **replegar a `pub(crate)`** o marcarlos `#[doc(hidden)]` como primitivas de
  test; (b) **hacerlos transaccionales**; (c) documentarlos como «solo test» y dejarlos.
- **Recomendación**: **(a)**. Una API pública que rompe el invariante nuclear del crate es una trampa
  con fecha de caducidad: funciona hasta que alguien la llama.

### (h) Los escritores de runtime no toman el lock

- **Origen**: **reserva del juez ciego de E25-H03**.
- **Qué es**: `persist_plan` y `write_receipt` escriben bajo `.lodestar/runtime/` **sin** el lock de
  publicación, mientras el barrido de temporales del GC (E24-H06) puede correr **desde otro proceso**.
  La ventana es estrecha y el daño acotado (un plan o un recibo que hay que reescribir, no un `.md`),
  y por eso E25-H03 se limitó a proteger el plano de **recuperación**, que es el que sí sostiene el
  invariante nuclear.
- **Recomendación**: cerrarlo si aparece un caso real, o cuando se toque el GC por otro motivo. Está
  registrado para que no se confunda con un olvido.

### (i) La secuencia de sellado está duplicada entre `apply` y `revert`

- **Origen**: **reserva del juez ciego de E25-H05**.
- **Qué es**: tras E25-H04/H05, publicar y revertir comparten la **misma coreografía** —promover el
  recibo pendiente, limpiar staging, borrar el journal, fsync del directorio— escrita **dos veces**.
  No es duplicación de *lógica de dominio* (invariante #3 no se incumple: la mecánica de recibo sí se
  reusa), pero sí de **secuencia**, que es donde un arreglo futuro se aplicará a una mitad y no a la
  otra. Es la forma exacta del defecto que E25-H05 vino a cerrar.
- **Recomendación**: extraer un `sellar_publicado(txn_id, journal_path)` compartido, en un ciclo
  corto y sin cambio de comportamiento, **con la suite actual como red**. Candidata clara a `/ciclo`.

### (j) Un cursor basura reinicia la paginación en silencio

- **Origen**: **reserva del juez ciego de E26-H10**.
- **Qué es**: `decode_cursor` interpreta un cursor ilegible como **offset 0**, así que un cursor
  corrupto o de otra tool devuelve **la primera página** en vez de un error. Un agente que pagina en
  bucle con un cursor mal propagado no termina nunca y no se entera. Es —en pequeño— el mismo patrón
  que E26-H08 acaba de retirar de la evaluación de consultas.
- **Por qué no se cerró en H10**: H10 tenía que introducir cotas **sin** cambiar el resultado de las
  llamadas correctas (`paginar_no_pierde_ni_duplica`); rechazar cursores inválidos es un caso de error
  **nuevo** en cuatro tools, o sea otro delta de contrato, en una historia que ya llevaba el suyo.
- **Recomendación**: `INVALID_SCHEMA` con mensaje, en el mismo ciclo que cualquier retoque futuro de
  paginación. Es barato y coherente con el principio rector de E26.

### (k) La matriz de trazabilidad no tiene filas de E15–E24

- **Origen**: **observación del cierre de E24-H18**, verificada al cerrar E25/E26.
- **Qué es**: `requirements/trazabilidad.md` se quedó en el giro headless (E9–E14). La migración a
  Markdown universal (E15–E22), el cierre de la PR #17 (E23) y el de la v0.3.0 (E24) **nunca se
  trazaron**, pese a que el alcance de E24-H18 lo declaraba («`requirements/README.md` y
  `requirements/trazabilidad.md` incorporan la épica»): el README **sí** se actualizó, la matriz no.
  Diez épicas sin fila. E25/E26 sí están trazadas, con lo que el hueco queda en medio y a la vista.
- **Por qué no se cerró aquí**: reconstruir la trazabilidad de diez épicas a posteriori es un trabajo
  de documentación con su propio alcance, y hacerlo a la carrera produciría filas plausibles en vez
  de filas verificadas — el defecto que el documento existe para impedir.
- **Recomendación**: una historia propia, con el criterio de que **cada fila se verifique contra la
  épica**, no contra el recuerdo.

### (l) Deuda de fuerza de suite y flecos menores registrados por los jueces ciegos

- **Origen**: las **reservas MENORES** de los veredictos de E25/E26. A diferencia de las mayores
  —que se cerraron en el mismo ciclo— y de (h), (i), (j), que son deuda de diseño, estas son de otra
  clase: **la suite no muerde ahí**. Casi todas salieron de *mutation testing*, no de un fallo
  observado, y se registran juntas porque comparten remedio.
- **Qué es**, por historia y con el mutante que lo destapó:
  - **E25-H01** — mutación **(g)**: el cálculo de `paths_divergentes` y el mensaje del conflicto de
    ventana pueden **vaciarse** sin que ningún test muerda. El aborto sigue ocurriendo (eso sí está
    cubierto), pero el diagnóstico que dice **qué** divergió no lo fija nadie.
  - **E25-H02** — mutación **S**: el sidecar de huellas movido a cuarentena se puede **borrar** sin
    que falle un test, pese a que la cuarentena existe precisamente para no perder material forense.
    Mutación **N**: la **numeración** de cuarentenas repetidas (`.2`, `.3`) tampoco está fijada, así
    que dos irrecuperables del mismo `txnId` podrían pisarse sin que se note.
  - **E25-H03** — mutación **c**: el **no-op silencioso** del GC dentro de `recover_if_pending` no
    tiene arnés. Está **mitigado** por el testigo tipado (`&WorkspaceLock` como prueba de que el lock
    se posee), que hace el error difícil de cometer de nuevo, pero mitigado no es cubierto.
  - **E25-H04** — mutante **k**: el guard `recibo_a_salvo` no tiene test que **inyecte un fallo de
    promoción**, así que la rama que decide qué hacer cuando el recibo no se pudo promover no se
    ejerce. Tiene **espejo** en el sellado del revert de H05.
  - **E25-H05** — dos: el wrapper `Workspace::revert_transaction` quedó **sin llamador ni test**
    propio; y la **re-verificación es única** en el revert mientras el apply comprueba **dos veces**,
    lo que deja declarada una ventana `[paso 2b, primer rename]` más ancha en el revert que en el
    apply. Es estrechamiento posible, no un agujero: la comprobación que importa —bajo el lock— sí
    está, y es la que E25-H05 introdujo.
  - **E26-H09** — **divergencia latente core↔store**: el catálogo publica ahora nombres **anclados**
    (`frontmatter.graph.backlinks`), mientras el store sigue indexando `metadata.field_path` con los
    nombres crudos de `walk`. **Hoy esa columna no la lee nadie**, así que no hay discrepancia
    observable — es la misma situación exacta de (c)/[`§14`](14-store-sin-consumidor.md), y se resolverá con ella.
  - **E26-H10** — la **aritmética de paginación está en 4 copias** y los límites se aplican en **3
    sitios por tool**. Es el vector del mutante **M10**, que se cerró clavando el default con un
    test; la duplicación sigue ahí, y es donde un arreglo futuro se aplicará a unas copias y no a
    otras — la misma forma que (i).
- **Por qué no se cerró**: ninguno es un defecto observable hoy. Cerrarlos uno a uno al vuelo, al
  final de once historias, habría añadido tests escritos para matar un mutante concreto en vez de
  para describir comportamiento — que es como se acumulan suites grandes y flojas. Y dos de ellos
  (H09, H10) no se arreglan con un test sino con el refactor que ya recomiendan (c) e (i).
- **Recomendación**: **una pasada de `/mutantes` acotada** a los ficheros que E25/E26 tocaron, con
  presupuesto cerrado, que convierta en test los supervivientes que describan comportamiento real
  —(g), S, N, k son los candidatos claros—; y, cuando se toque ese código por otro motivo, los dos
  refactores compartidos que ya están recomendados: **`sellar_publicado`** (i) y un **helper único de
  paginación** para la aritmética y los límites. La divergencia core↔store de H09 **no se toca aquí**:
  va con la decisión de [`§14`](14-store-sin-consumidor.md).
