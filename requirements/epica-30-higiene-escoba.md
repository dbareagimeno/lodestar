# E30 — Ciclo de higiene y escoba de la campaña de bugfixes

> **Origen**: Fases 2 y 3 de [`docs/qa/campana-bugfixes-2026-08.md`](../docs/qa/campana-bugfixes-2026-08.md)
> (revisado 2026-08-07), que alimenta de [`decisiones/16-deuda-auditoria-e25-e26.md`](../decisiones/16-deuda-auditoria-e25-e26.md)
> subpunto (j), [`decisiones/23-hallazgos-testbench-homelab.md`](../decisiones/23-hallazgos-testbench-homelab.md)
> filas A-02/A-03/D-02/A-01/A-06/A-09/A-10 y el informe reproducible
> [`docs/qa/informe-homelab-2026-08-06.md`](../docs/qa/informe-homelab-2026-08-06.md) (casos ROB-05,
> ROB-06). Es la **Fase 2 + Fase 3** de la campaña; **E28** fue la Fase 0 (defectos destructivos) y
> **E29** la Fase 1 (honestidad de superficie), las dos completas y pendientes de merge.
>
> **Objetivo de la épica**: cerrar lo que queda abierto en la tabla de la campaña tras E28/E29: (1)
> un cursor de paginación malformado o de otra tool deja de reinterpretarse en silencio como offset
> 0 — `§16(j)` ampliado con la variante cross-tool que el testbench añadió (A-02/A-03); (2) la
> flakiness real de `crash_por_senal_no_deja_parciales`, señalada por tres jueces ciegos distintos,
> se diagnostica hasta la causa raíz y se cierra (no se enmascara con reintentos); (3) la escoba
> documental de los cinco nits originales de `§23` (D-02, A-01, A-06, A-09, A-10) más los
> seguimientos nuevos que los jueces ciegos de E28/E29 registraron sin numerar como punto propio.
> Referencias maestras: `ARCHITECTURE.md §19.5` (modelo transaccional y sus garantías), `§20.4`
> (edición de frontmatter), `§20.8`/`§20.9` (lenguaje de consulta y diagnósticos), `contracts/mcp.yml`,
> `CLAUDE.md` (invariantes).

**Principio rector**: el corolario de E26 que ya gobierna toda la campaña — *una respuesta
silenciosamente equivocada es peor que un error* — aplicado esta vez a la propia **suite**, no solo
al wire: un test flaky que "pasa" por una carrera no verificada es la misma clase de mentira que un
cursor que reinterpreta silenciosamente. Y su corolario documental: donde el comportamiento ya es
correcto y solo falta que conste, la escoba **declara**, no cambia — H03 no introduce comportamiento
nuevo salvo donde el propio informe lo pide explícitamente (D-02 ya tiene criterio ratificado;
`protocolVersion` no-string es la única línea de código nueva del lote de nits).

**Fuera de alcance (explícito)**: esta épica **no** reabre ninguna ficha que ejecuta, y en particular
deja fuera:

- **`§16(l)`** (pasada de `/mutantes` acotada a E25/E26) — es la otra mitad de la Fase 2 del orden de
  trabajo, pero es un ciclo de trabajo distinto (mutation testing, no historias de spec) y no
  comparte forma con H01/H02/H03. Queda para su propio turno, fuera de esta épica.
- **`§21`** (comillas del lenguaje de consulta) y las tres limitaciones de *quoting* que motivan esa
  ficha — necesitan sintaxis nueva y puerta de diseño propia.
- **`§14`** (store sin consumidor), **`§9`** (banco de pruebas), **`§20`** (renombrado), **`§22`**
  (integridad referencial), **`§16(h)`** (escritores de runtime sin lock), **`§16(k)`** (matriz de
  trazabilidad) y **`decisiones §24`** (equivalencia de paths por caja/Unicode).
- La familia de «normalización con estado de contenido acumulado» (resurrección de paths liberados
  por operaciones de contenido tras `delete`/`move`, y move-chains por ocupación del origen) — es
  hallazgo registrado por E28, con dueño propio, y **no** comparte zona con ninguna de las tres
  historias de aquí (paginación, lock, documentación).
- Cualquier **capacidad nueva** de consulta o de wire: H01 formaliza la semántica de un cursor que ya
  existe (no añade paginación donde no la había); H03 documenta o pone un guard de una línea, nunca
  sintaxis nueva.

---

## Mapa de la épica

| ID | Origen | Título corto | Frontera | Modelo sugerido |
|---|---|---|---|---|
| E30-H01 | `§16(j)` + `§23/A-02,A-03` | Cursores estrictos: malformado o ajeno es `INVALID_SCHEMA` | sí | opus |
| E30-H02 | Observación campaña (3 jueces) | Flakiness de `crash_por_senal_no_deja_parciales` | no | opus |
| E30-H03 | `§23/D-02,A-01,A-06,A-09,A-10` + seguimientos E28/E29 | Escoba documental y nits | parcial | sonnet |

**Orden de construcción**: `H01` → `H02` en paralelo con `H01` (zonas de código y garantías
distintas: paginación de lectura en `lodestar-app::pagina`/`decode_cursor` vs. concurrencia de
publicación en `lodestar-workspace::lock`) → `H03` al final, porque su lista de nits incluye follow-ups
que solo tienen sentido leer tras H01/H02 (para no documentar un comportamiento que H01 está a punto
de cambiar).

---

## E30-H01 — Cursores estrictos: malformado o ajeno a la tool es `INVALID_SCHEMA`

- **Objetivo**: un cursor de paginación que no se puede decodificar, o que es sintácticamente válido
  pero fue emitido por **otra** tool o **otra** consulta, deja de reinterpretarse en silencio como
  offset 0 (o como una página válida pero semánticamente ajena) y produce `INVALID_SCHEMA` nombrando
  el problema. Un cursor legítimo — el `nextCursor` que la misma tool devolvió en una respuesta
  anterior de la misma consulta — sigue paginando exactamente igual que hoy.

- **Síntoma reproducible**:
  - **A-02 (ROB-05, `decisiones §23`)**: un cursor malformado como `"zzz-no-hex"` cae a offset 0 en
    silencio. Causa raíz: `decode_cursor` (`crates/lodestar-app/src/lib.rs:3987-3989`) —
    `usize::from_str_radix(cursor, 16).unwrap_or(0)` — cualquier cadena que no parsea como hex se
    trata como «empieza desde el principio», indistinguible de un cliente que omite `cursor` a
    propósito.
  - **A-03 (ROB-06, `decisiones §23`)**: un `nextCursor` que devolvió `graph_query` es **aceptado**
    por `knowledge_search` y reinterpretado como offset propio de esa consulta distinta: página
    "válida" en forma, semánticamente equivocada — el agente cree haber avanzado en la búsqueda que
    pidió y en realidad ve un fragmento de otro resultado. Causa raíz: las cuatro tools paginadas
    (`knowledge_search`, `knowledge_check`, `metadata_inspect`, `graph_query`) comparten literalmente
    el mismo `pagina()`/`encode_cursor()`/`decode_cursor()` (`lib.rs:4000-4018` y su cabecera,
    `§20.10`/`E26-H10`: *"el mismo cursor-offset hex autosuficiente en toda la superficie"*) — el
    cursor no lleva ninguna marca de qué tool o qué consulta lo produjo, así que **por construcción**
    no hay forma de detectar que viene de otro sitio: cualquier hex válido de cualquier tool decodifica
    a un offset que las cuatro aceptan sin más.

- **Referencias**: `ARCHITECTURE.md §19.6` (superficie de las 10 tools y su paginación) · `§20.10`
  (paginación por cursor, E26-H10) · `contracts/mcp.yml` (líneas `cursor: tipo: string, semantica:
  "cursor opaco..."` de `knowledge_search`/`knowledge_check`/`metadata_inspect`/`graph_query`; el
  contrato llama al cursor **"opaco"** mientras la codificación real —offset hex compartido— es
  pública por construcción, la contradicción que `decisiones §23/A-03` señala explícitamente) ·
  `crates/lodestar-app/src/lib.rs` (`decode_cursor:3987`, `encode_cursor:3982`, `pagina:4000`) ·
  `crates/lodestar-mcp/src/tools.rs` (los cuatro sitios que leen `cursor` vía `str_validado`: L314,
  L365, L396, L432) · `decisiones §16(j)` (decidido: `INVALID_SCHEMA`, ampliado por
  `decisiones §23` fila 4 con la variante cross-tool) · `CLAUDE.md` invariante #4 (un solo contrato
  de tipos: el cursor no puede tener una forma en el contrato — "opaco" — y otra en el código —
  "offset hex sin firma"— sin que una de las dos mienta).

- **Alcance**:
  - **Cursor firmado con su origen**: el cursor deja de ser un offset hex desnudo y pasa a llevar,
    codificado junto al offset, un identificador de la **tool** y de la **consulta/modo** que lo
    produjo — lo suficiente para que decodificar un cursor ajeno falle de forma determinista en vez
    de "colar" un offset numéricamente válido. La forma concreta (un prefijo por tool + separador, un
    hash corto de los parámetros de consulta que participan en el orden total, o un esquema
    equivalente) la fija la fase roja; el criterio no es la forma interna, es el comportamiento
    observable de los tres casos de abajo. Se mantiene la propiedad que ya declara el rustdoc de
    `encode_cursor` — *"autosuficiente: un offset reanuda idénticamente en cualquier servidor
    fresco"* — para la tool y la consulta correctas: no se introduce estado de sesión ni un cursor
    que deje de servir tras reiniciar el proceso.
  - **`decode_cursor` deja de ser infalible**: pasa a devolver `Result` (o el tipo que la fase roja
    fije) en vez de `unwrap_or(0)`. Un cursor que no decodifica —sea basura sintáctica (A-02) o de
    forma válida pero de origen distinto (A-03)— propaga un error hasta la fachada MCP, que lo sirve
    como `INVALID_SCHEMA` **nombrando el problema**: qué cursor se recibió y, si es determinable, qué
    tool lo esperaba frente a cuál lo produjo realmente (mismo estilo que el resto del catálogo de
    type errors, `E26-H07`/`E29-H04`).
  - **`pagina()` sigue siendo el único punto de mecánica de paginación** (invariante #3/#4: no se
    duplica la lógica en las cuatro tools) — gana el parámetro de identidad de tool/consulta que
    necesita para fabricar y verificar el cursor, sin que las cuatro fachadas reimplementen nada.
  - **Un cursor de la misma tool pero de una consulta *distinta* dentro de esa tool** (p. ej. dos
    llamadas a `knowledge_search` con `where` diferente) es del mismo género que A-03: la fase roja
    decide si la identidad de consulta se ata también al criterio de orden (`where`/`filter`/`text`)
    o si la historia se limita a la identidad de **tool**, dejando la de consulta como hallazgo de
    seguimiento — pero **debe** decidirlo explícitamente y dejarlo escrito, no dejarlo a
    interpretación de quien lo descubra después.
  - `docs/user/` (si algún documento de usuario describe el cursor como "opaco" sin matiz) se ajusta
    a la nueva semántica en la misma pasada, junto con la corrección del delta de contrato.

- **Fuera de alcance**: cualquier cambio en **qué se pagina** o en el **orden total** de cada tool
  (eso sigue siendo determinista y depende solo del contenido, como hoy); introducir cursores con
  estado de sesión o con TTL — el cursor sigue siendo autosuficiente; cambiar el `limit`/`default`/
  `max` de ninguna tool (`E26-H10` ya los fijó); la identidad de **paginación** de `change_plan` con
  selección masiva, que no usa este mecanismo.

- **Criterios de aceptación**:
  - **Dado** una llamada a `knowledge_search` con `cursor: "zzz-no-hex"`, **Cuando** se ejecuta,
    **Entonces** la respuesta es `INVALID_SCHEMA` nombrando el cursor recibido como no decodificable
    (no una página desde el offset 0) → **test: `cursor_malformado_es_invalid_schema`**.
  - **Dado** un `nextCursor` devuelto por una llamada real a `graph_query`, **Cuando** se pasa ese
    mismo valor como `cursor` a `knowledge_search`, **Entonces** la respuesta es `INVALID_SCHEMA`
    nombrando que el cursor no pertenece a esta tool (no una página "válida" de `knowledge_search`)
    → **test: `cursor_de_otra_tool_es_invalid_schema`**.
  - **Dado** un `nextCursor` devuelto por `metadata_inspect` en modo `catalog`, **Cuando** se pasa a
    `metadata_inspect` en modo `field`, **Entonces** también es rechazado (misma tool, contexto
    distinto dentro de ella — el criterio decidido en la fase roja para este caso se ejerce con
    test, sea cual sea) → **test: `cursor_de_otro_modo_de_la_misma_tool_segun_decision_de_fase_roja`**.
  - **Dado** un workspace con más documentos que el `limit` de página, **Cuando** se pide la primera
    página de `knowledge_search`, se toma su `nextCursor`, y se usa **ese** valor exacto en una
    segunda llamada a `knowledge_search` con los **mismos** parámetros de consulta, **Entonces** la
    segunda página se sirve correctamente y sin solaparse con la primera → **test:
    `cursor_legitimo_de_la_misma_tool_sigue_paginando`** (anti-vacuo: el rechazo de A-02/A-03 no
    puede romper el camino feliz — el criterio se ejerce con un cursor **obtenido de una respuesta
    real**, nunca fabricado a mano, para no dar por bueno un roundtrip vacuo).
  - **Dado** ese mismo recorrido completo (primera página → cursor → segunda página → … hasta
    `nextCursor: null`), **Cuando** se concatenan todas las páginas, **Entonces** el conjunto
    coincide exactamente con el que produciría la tool sin paginar (mismo orden total, sin
    duplicados ni huecos) → **test: `roundtrip()` de recorrido completo por cursor legítimo**, uno
    por cada una de las cuatro tools paginadas.
  - **Dado** el par `encode_cursor`/`decode_cursor` tras el arreglo, **Cuando** se codifica un offset
    y se decodifica de vuelta con la identidad de tool/consulta correcta, **Entonces** el offset
    recuperado es exactamente el original → **test unitario: `roundtrip()`** del núcleo de
    codificación en `crates/lodestar-app` (nombre del arnés existente reutilizado a propósito: es el
    mismo patrón "codifica→decodifica→compara" que ya usa el resto de la suite para este tipo de
    invariante).
  - **Dado** `contracts/mcp.yml`, **Cuando** se revisa la semántica del parámetro `cursor` en las
    cuatro tools, **Entonces** ya no lo llama "opaco" sin matiz y declara que un cursor de otra tool
    o malformado es `INVALID_SCHEMA` → checklist estructural (revisión de diff).

- **Dependencias**: ninguna técnica. Paralelizable con `E30-H02` (zonas de código disjuntas:
  `lodestar-app`/`lodestar-mcp` de lectura vs. `lodestar-workspace::lock` de publicación).

- **Pruebas**: tests unitarios de `crates/lodestar-app/src/lib.rs` (`#[cfg(test)]`, el par
  `encode_cursor`/`decode_cursor` y `pagina()` con identidad de tool) + `roundtrip()` de
  `crates/lodestar-mcp/tests/mcp.rs` (los tres casos BDD por el wire real: malformado, cross-tool,
  legítimo con recorrido completo) + un caso en `crates/lodestar-mcp/tests/descubribilidad.rs` si el
  delta de contrato lo exige verificar contra el catálogo declarado. Fixtures: `lodestar-fixtures`
  con suficientes documentos para forzar más de una página en cada una de las cuatro tools (el mismo
  fixture que ya usa `E26-H10` para sus tests de cota, reutilizado).

- **Delta de contrato propuesto** (`contracts/mcp.yml`):
  - Las cuatro entradas de parámetro `cursor` (`knowledge_search`, `knowledge_check`,
    `metadata_inspect`, `graph_query`) dejan de decir solo *"cursor opaco... devuelto en
    nextCursor"* y ganan la advertencia: un cursor que no decodifica, o que fue emitido por otra tool
    o consulta, es `INVALID_SCHEMA` — no se reinterpreta como offset 0 ni se acepta como página de
    otro contexto.
  - Las listas `errores:` de las cuatro tools ganan (o extienden, si ya citan `INVALID_SCHEMA` por
    otro motivo) la entrada correspondiente al cursor inválido/ajeno.
  - Sin código de error nuevo: `INVALID_SCHEMA` ya está en el catálogo de 17 filas.
  - Si la forma elegida en la fase roja cambia la longitud o el alfabeto del cursor de wire (por
    ejemplo, deja de ser hex puro), la nota de que es "hexadecimal" en la cabecera del contrato
    (línea ~217) se corrige para no mentir sobre el formato — la propiedad que **sí** debe
    mantenerse es que sigue siendo una cadena opaca desde la perspectiva de un cliente que no la
    fabrica a mano, solo que ahora "opaco" es verdad de verdad.

- **Proceso**: ciclo **completo** con **panel** de jueces (toca `contracts/mcp.yml` en las cuatro
  tools de mayor superficie de la API y cambia el comportamiento observable de un parámetro que
  todo cliente que pagina ya usa: el riesgo de regresión silenciosa en el camino feliz es alto, y es
  exactamente donde el criterio anti-vacuo de recorrido completo por cursor real debe verificarse con
  más cuidado).

---

## E30-H02 — Flakiness de `crash_por_senal_no_deja_parciales`: diagnóstico y arreglo de causa raíz

- **Objetivo**: `crash_por_senal_no_deja_parciales`
  (`crates/lodestar-mcp/tests/crash_senal.rs`) deja de fallar de forma intermitente bajo
  `cargo test --workspace`. La historia exige encontrar la **causa raíz real** de la
  intermitencia — no enmascararla con reintentos ciegos ni con un `sleep` más largo — y aplicar el
  arreglo en el lugar que corresponda: en el motor, si la reclamación de lock huérfano tiene un
  defecto real bajo carga; en el arnés del test, si la intermitencia es un artefacto legítimo de
  cómo el test comparte recursos con el resto de la suite en paralelo.

- **Síntoma reproducible**: tres jueces ciegos distintos (`docs/qa/campana-bugfixes-2026-08.md`,
  observación de cierre de Fase 0/1) registraron el mismo fallo intermitente: el test falla en
  torno al 50% de las ejecuciones bajo `cargo test --workspace` (con el resto de la suite corriendo
  en paralelo, incluidos otros tests que abren procesos `lodestar-mcp` reales), con el mensaje `el
  lock de publicación ya está tomado (…): WRITE_CONFLICT`, y **pasa de forma consistente** cuando se
  ejecuta en aislamiento (`cargo test -p lodestar-mcp --test crash_senal
  crash_por_senal_no_deja_parciales`).

- **Sospecha registrada** (a verificar, no a asumir como conclusión): el test mata el proceso
  `lodestar-mcp` con `SIGKILL` a mitad de un `change_apply`
  (`crates/lodestar-mcp/tests/crash_senal.rs:100-108`, método `Sesion::matar`) y a continuación abre
  un **segundo** proceso sobre el **mismo** directorio temporal (`Sesion::abrir`, línea 222) que debe
  poder tomar el lock de publicación que el proceso muerto dejó. La reclamación de un lock huérfano
  vive en `crates/lodestar-workspace/src/lock.rs`, función `reclamar_si_huerfano` (líneas ~237-294):
  decide si el dueño anterior está muerto con `vida_del_dueño` (línea ~323, `libc::kill(pid, 0)`) y
  solo reclama con certeza si el `host` del lock coincide con el host local (`es_host_local`, línea
  ~262) — en cualquier otro caso, incluida una prueba de vida "desconocida", cae al TTL de 15 minutos
  (`LOCK_TTL`, línea 198), que en un test **no** debería ni acercarse a vencer. La sospecha concreta:
  bajo la carga de `--workspace` (docenas de procesos y hilos compitiendo por CPU/IO), la ventana
  entre que el `SIGKILL` mata al proceso y que el segundo `Sesion::abrir` intenta adquirir el lock
  puede no ser suficiente para que el pid muerto **ya no exista** de forma observable por
  `libc::kill(pid, 0)` en el primer intento (reciclado de pid bajo presión, *scheduling* retrasado
  del `wait()` del SO, o alguna ventana equivalente) — y `acquire_lock`
  (`crates/lodestar-workspace/src/lock.rs:123-188`) **no reintenta**: un solo intento de
  `reclamar_si_huerfano` que resuelve `Vida::Desconocida`/`Vida::Viva` por mala suerte de *timing*
  deja el `WriteConflict` como error final, sin una segunda oportunidad dentro de la misma llamada.

- **Referencias**: `ARCHITECTURE.md §19.5` (modelo transaccional: lock de publicación, escritor
  único) · `crates/lodestar-workspace/src/lock.rs` (`acquire_lock:123`,
  `reclamar_si_huerfano:237`, `vida_del_dueño:323`, `es_host_local:354`, `LOCK_TTL:198`) ·
  `crates/lodestar-mcp/tests/crash_senal.rs` (`Sesion::matar:101`, `Sesion::abrir:40`,
  `crash_por_senal_no_deja_parciales:180`) · `decisiones §25-H06` (histórico: la prueba de vida por
  pid que este defecto pone a prueba bajo carga) · `docs/qa/campana-bugfixes-2026-08.md` (registro
  de los tres jueces) · `CLAUDE.md` invariante #5 (único escritor: el lock es el mecanismo que lo
  sostiene, así que su fiabilidad bajo carga real importa tanto como su corrección en el camino
  feliz).

- **Alcance**:
  - **Diagnóstico primero, arreglo después**: antes de tocar una línea de producción o de test, la
    historia exige instrumentar y **reproducir de forma controlada** la carrera sospechada —por
    ejemplo, ejecutando el test repetidamente bajo carga artificial (`cargo test --workspace` real,
    o un harness que sature CPU/IO mientras el test corre) y registrando, en cada fallo, qué
    devuelve exactamente `reclamar_si_huerfano` (`Reclamado` vs. `Vivo(detalle)`) y qué rama de
    `vida_del_dueño`/`es_host_local` se tomó. El diagnóstico debe quedar escrito (en el commit de
    spec de la fase roja o en el propio código como comentario, no solo en la cabeza de quien lo
    encontró) antes de decidir dónde va el arreglo.
  - **Si la causa raíz es un defecto real de reclamación** (p. ej. una ventana de *timing* entre
    `SIGKILL` y la observabilidad del pid muerto que hace que `vida_del_dueño` devuelva algo distinto
    de `Muerta` en el primer intento, de forma reproducible bajo carga): el arreglo vive en
    `lodestar-workspace::lock`, y debe preservar los invariantes que `reclamar_si_huerfano` ya
    garantiza (ante la duda no se reclama; un dueño vivo nunca pierde su lock) — no se relaja el
    criterio de "vivo", se hace más fiable la comprobación (por ejemplo, un reintento acotado con
    backoff dentro de la misma llamada a `acquire_lock`, o un criterio de vida que no dependa de una
    única lectura puntual de `libc::kill`). Cualquier reintento que se introduzca debe tener un tope
    determinista (no un bucle sin límite) para no convertir un `WriteConflict` legítimo (lock de un
    escritor realmente vivo) en una espera indefinida.
  - **Si la causa raíz es un artefacto del arnés de test** (p. ej. el test comparte recursos de forma
    que no representa un lock huérfano real — un directorio temporal reutilizado, una condición de
    carrera en el propio test entre `matar()` y `abrir()` que no existe en el uso real del motor): el
    endurecimiento vive en `crash_senal.rs`, y debe seguir ejerciendo el mismo invariante nuclear
    (ni un `.md` a medias, convergencia a uno de los dos bordes) sin degradar su cobertura — no vale
    "arreglar" la flakiness quitándole al test la capacidad de golpear la ventana real de crash que
    `E24-H14` documenta como su razón de ser.
  - **Prohibido explícitamente**: envolver la aserción en un retry ciego (`for _ in 0..N { if ok
    { break } }`) sin haber diagnosticado antes qué falla y por qué esa repetición lo arregla. Un
    retry es aceptable **como parte del arreglo** solo si el diagnóstico establece que la
    intermitencia es inherente a una ventana de *timing* del sistema operativo que ni el motor ni el
    test pueden eliminar del todo (análogo a por qué `crash_por_senal_no_deja_parciales` ya escalona
    varios retrasos de `SIGKILL` en vez de fijar uno solo) — y en ese caso el propio código deja
    escrito por qué el retry es legítimo, no un parche.

- **Fuera de alcance**: cualquier cambio en la semántica de `LOCK_TTL`, en el criterio de "vivo" para
  un host **remoto** (`es_host_local` devolviendo `false`), o en la prueba de vida en plataformas no
  Unix (`vida_del_dueño` con `#[cfg(not(unix))]`) — el defecto reportado es específico de la carrera
  local bajo carga, no de la lógica cross-host o cross-plataforma, que E25-H06 ya endureció con su
  propio criterio y no se relitiga aquí; ningún otro test de la suite (el resto de
  `crash_senal.rs`, incluidos `crash_tras_publicar_deja_transaccion_reversible` y
  `crash_durante_revert_deja_inversa_reversible`, que hoy no están en la lista de flaky).

- **Criterios de aceptación**:
  - **Dado** el diagnóstico de la fase roja, **Cuando** se documenta la causa raíz, **Entonces** el
    documento (spec o comentario en código) identifica la rama exacta de `reclamar_si_huerfano`/
    `vida_del_dueño` que falla bajo carga, o describe el artefacto del arnés, con evidencia
    reproducible (no una hipótesis sin verificar) → revisión de diff (criterio estructural, previo a
    cualquier cambio de código).
  - **Dado** el arreglo aplicado (en motor o en arnés, según lo que el diagnóstico exija),
    **Cuando** se ejecuta `cargo test --workspace --locked` de forma consecutiva **N veces seguidas**
    (N = 20, elegido para que la probabilidad de que una tasa de fallo del 50% pase 20 veces seguidas
    por azar sea despreciable: `0.5^20 ≈ 1×10⁻⁶`), **Entonces** ninguna de las 20 ejecuciones falla en
    `crash_por_senal_no_deja_parciales` → **criterio de aceptación explícito, no un test unitario**:
    se ejecuta y su resultado (log de las 20 corridas) se adjunta a la verificación de la historia.
  - **Dado** el mismo arreglo, **Cuando** se ejecuta el test en aislamiento
    (`cargo test -p lodestar-mcp --test crash_senal crash_por_senal_no_deja_parciales`), **Entonces**
    sigue pasando exactamente igual que antes del arreglo (control anti-vacuo: el arreglo no puede
    depender de que el test corra solo) → verificación explícita, mismo criterio que el punto anterior.
  - **Dado** el resto de tests de `crash_senal.rs`
    (`crash_tras_publicar_deja_transaccion_reversible`, `crash_durante_revert_deja_inversa_reversible`)
    y el resto de la suite que abre procesos `lodestar-mcp` reales bajo `--features test-failpoints`,
    **Cuando** se ejecutan tras el arreglo, **Entonces** siguen pasando sin cambio de comportamiento
    → control anti-vacuo: ningún otro test de crash/recuperación regresiona.
  - **Dado** `docs/qa/campana-bugfixes-2026-08.md`, **Cuando** se actualiza tras el cierre,
    **Entonces** `crash_por_senal_no_deja_parciales` deja de estar listado como candidato a
    flakiness/higiene, con una nota que registra la causa raíz encontrada (para que la próxima
    auditoría no lo redescubra como hallazgo nuevo — la misma disciplina que `decisiones §16`) →
    checklist estructural (revisión de diff del documento).

- **Dependencias**: ninguna técnica. Paralelizable con `E30-H01` (zonas de código disjuntas).

- **Pruebas**: el propio `crates/lodestar-mcp/tests/crash_senal.rs` (sin nuevos ficheros de test:
  el criterio es sobre la **fiabilidad** de un test ya existente, no sobre comportamiento nuevo que
  necesite un test nuevo) + si el arreglo vive en el motor, tests unitarios de
  `crates/lodestar-workspace/src/lock.rs` (`#[cfg(test)]`) que fijen el caso de carrera diagnosticado
  de forma determinista (sin depender de la suerte de `cargo test --workspace`, para que quede una
  red de seguridad reproducible bajo demanda, no solo la ejecución de 20 veces). Fixtures: ninguna
  nueva — el escenario ya existente de `crash_senal.rs` es suficiente.

- **Frontera (mcp.yml)**: no. Es un defecto de fiabilidad interna del lock de publicación, invisible
  desde el wire (un cliente real nunca ve la carrera de tests en paralelo).

- **Proceso**: ciclo **completo**, pero de forma distinta a las historias de comportamiento: la fase
  roja **es** el diagnóstico (reproducir la carrera de forma controlada, no escribir un test que
  falla), y la fase verde es el arreglo verificado con las 20 ejecuciones consecutivas. El juez ciego
  debe poder verificar que el diagnóstico escrito es coherente con el arreglo aplicado —que no se
  arregló una cosa distinta de la que se diagnosticó— y que el criterio de las 20 ejecuciones se
  ejecutó de verdad, no se asumió.

---

## E30-H03 — Escoba documental y nits (Fase 3)

- **Objetivo**: los cinco nits originales de `decisiones §23` (D-02, A-01, A-06, A-09, A-10) y los
  seis seguimientos que los jueces ciegos de E28/E29 registraron sin numerar como punto nuevo quedan
  saldados: donde el comportamiento ya es correcto y solo falta documentarlo, se documenta; donde hay
  un hueco real de cobertura o una línea de arreglo barato ya identificada, se cierra con test de
  guardia o con el arreglo de una línea que la propia campaña ya localizó.

- **Alcance — los cinco nits de `§23`** (cada uno con su criterio):

  1. **D-02 — `patch_frontmatter`: `ARCHITECTURE.md §20.4` promete distinguir asignar-`null` de
     eliminar-clave; el wire real es RFC 7386 puro** (**criterio ya ratificado por el usuario el
     2026-08-06**: corregir `§20.4`, declarar merge-patch RFC 7386). Cuatro sitios exactos a
     corregir, todos afirmando hoy una semántica "null-borra" que el wire de MCP no puede expresar
     (el `Some(Null)` del core es inalcanzable desde `patch_frontmatter`, que serializa como merge-patch):
     - `ARCHITECTURE.md:227` — `merge_frontmatter(&self, p: &RelPath, patch: FrontmatterPatch) ->
       WriteOutcome; // null borra` → el comentario deja de decir "null borra" y declara la
       semántica RFC 7386 (`null` en el patch **elimina** la clave, no la asigna a `null` — es
       exactamente lo que RFC 7386 define, así que la corrección es de **redacción**, no de código).
     - `ARCHITECTURE.md:400` — `update_frontmatter`(validado, patch con null-borra) → mismo ajuste
       de redacción.
     - `ARCHITECTURE.md:515` — fila de la tabla histórica *"`merge_frontmatter` (patch, null-borra)
       no existía en el core"* → se corrige para que la nota histórica siga siendo cierta como
       registro de esa migración sin inducir a leer "null-borra" como si fuera una capacidad
       *distinta* de RFC 7386 que el wire promete y no cumple.
     - `ARCHITECTURE.md:850-852` — *"reutiliza `FrontmatterPatch` (merge-patch RFC 7386,
       null-borra)"* → el paréntesis se reformula para que "null-borra" quede descrito como **la
       semántica que RFC 7386 ya define** (asignar `null` a una clave en el patch la elimina), no
       como una capacidad adicional de distinguir "asignar null" de "eliminar".
     - **Sección `§20.4` completa** (`ARCHITECTURE.md:1098-1101`, *"Edición de frontmatter"*): la
       frase *"distingue explícitamente asignar `null` de eliminar una clave"* se sustituye por la
       declaración de que `patch_frontmatter` sigue **merge-patch RFC 7386**: una clave ausente del
       patch no se toca, una clave presente con valor `null` se **elimina**, y no existe una forma
       de asignar literalmente `null` a una clave por esta vía (si un usuario necesita ese valor,
       debe pasarlo como cualquier otro escalar — el propio RFC no lo permite, y ampliar el wire para
       este caso queda fuera, tal como recomienda la ficha, "solo si aparece un caso real").
     - **Criterio de aceptación**: **Dado** el texto de `ARCHITECTURE.md §20.4` y las cuatro citas
       cruzadas tras la corrección, **Cuando** se busca la frase "null-borra" o "distingue... null...
       de eliminar", **Entonces** ya no aparece ninguna que prometa una distinción que el wire no
       ejecuta → checklist estructural (grep + revisión de diff).

  2. **A-01 — `knowledge_get.sections` con un `headingPath` sin match lo omite en silencio**: hoy
     solo lo documenta el doc-comment de `core::model::extract_sections`
     (`crates/lodestar-core/src/model.rs:852`). Se documenta en `contracts/mcp.yml` (semántica de
     `knowledge_get`, parámetro `sections`) y en la fuente de usuario correspondiente
     (`docs/user/`, si existe una sección sobre `knowledge_get`): un `headingPath` que no casa
     ningún heading se omite del resultado sin error; si **ninguno** de los `headingPath` pedidos
     casa, `body` es la cadena vacía — comportamiento defendible (no se decide ruido, `§23` ya lo
     evalúa como tal), pero deja de estar solo en un comentario de Rust.
     - **Criterio de aceptación**: **Dado** `contracts/mcp.yml`, **Cuando** se busca la semántica de
       `sections` en `knowledge_get`, **Entonces** declara la omisión silenciosa de headings sin
       match → checklist estructural (revisión de diff).

  3. **A-06 — `replace_text` con `find` sin ocurrencias y sin aserción produce un plan no-op
     silencioso** (`canApply: true`, diff vacío): `safe-changes.md` solo documenta el vacío-sin-error
     para **selecciones masivas**; en forma-array (`operations: [{...}]`) no está fijado como
     comportamiento declarado. Se documenta en `docs/user/safe-changes.md` (o el fichero equivalente
     que ya cubre selección masiva, extendiendo su alcance a la forma-array) **y** se fija con un
     test de guardia que congele el comportamiento (plan no-op con `canApply: true` y diff vacío,
     para que un cambio futuro que lo convierta en error o en un plan con contenido lo note).
     - **Criterio de aceptación**: **Dado** un documento sin ninguna ocurrencia de un `find` dado,
       **Cuando** se llama a `change_plan` con `replace_text` en forma-array sin
       `expectedOccurrences`, **Entonces** el plan resultante tiene `canApply: true` y un diff vacío
       para ese documento (sin error) → **test: `replace_text_sin_ocurrencias_en_forma_array_es_noop`**.

  4. **A-09 — la config se lee una vez por sesión** (`Workspace::open`): un `config.yaml` escrito con
     el servidor vivo no se aplica hasta reabrir. Hoy solo lo fija un comentario de `lib.rs:116`.
     `contracts/mcp.yml` lista `INTERNAL_IO_ERROR (carga de config)` **por llamada**, lo que sugiere
     (sin decirlo) que se relee — se declara explícitamente el ciclo de vida: la config se carga
     **una vez**, al abrir el workspace, y no se recarga en caliente durante la sesión.
     - **Criterio de aceptación**: **Dado** `docs/user/` (la fuente que describe `config.yaml`) y
       `contracts/mcp.yml`, **Cuando** se busca cuándo se lee la config, **Entonces** declara
       explícitamente que es una vez por apertura de workspace/sesión → checklist estructural
       (revisión de diff).

  5. **A-10 — `mcp.yml` L149-151 impreciso sobre `workspaceDirectory`**: el texto afirma que *"un
     path que normaliza a un directorio (`../docs/..`)"* da `workspaceDirectory`, cuando en realidad
     solo ocurre si normaliza a la **raíz**; un path que normaliza a un directorio con nombre propio
     da `missing`. Se corrige la redacción para que sea precisa.
     - **Criterio de aceptación**: **Dado** `contracts/mcp.yml` L149-151 tras la corrección,
       **Cuando** se lee, **Entonces** distingue explícitamente "normaliza a la raíz" de "normaliza a
       un directorio con nombre" → checklist estructural (revisión de diff).

- **Alcance — los seis seguimientos acumulados de los jueces de E28/E29**:

  6. **Familia `contains` con literal no-string sobre un campo string**: `eval_contains`
     (`crates/lodestar-core/src/eval.rs:341-362`, brazo `V::String(texto) => Ok(match literal {
     QueryValue::String(aguja) => texto.contains(aguja), _ => false })`) devuelve `false` en
     silencio cuando el campo **es** string pero el literal comparado **no** lo es (p. ej. `titulo
     contains 3`) — mismo patrón que A-04 (`starts_with`/`ends_with`) pero para `contains`, y fuera
     del alcance que fijó `E29-H04` (que solo tocó el caso "campo no-string"). La historia **propone**
     la decisión: por coherencia directa con el criterio ya ratificado de A-04 (*"la coherencia del
     lenguaje vale más que la compatibilidad con un comportamiento que ningún test fijaba"*), este
     caso debería tratarse igual — type error, no `false` silencioso —, pero **no lo decide aquí**:
     es una decisión de comportamiento de wire (abre `TypeError`, puede cambiar resultados de
     consultas existentes), y `decisiones/README.md` no la ha tomado como tal. **Alcance mínimo de
     esta historia**: documentar el hueco en `docs/user/query-language.md` junto a los demás type
     errors ya descritos, dejando explícito que es un caso **no cubierto** (no que sea intencional).
     Si el usuario ratifica extenderlo a type error en la misma pasada, se convierte en criterio de
     comportamiento con test; si no, queda como nit documental y candidato de una futura historia de
     comportamiento — el criterio de aceptación de abajo cubre ambos desenlaces.
     - **Criterio de aceptación (documental, mínimo)**: **Dado**
       `docs/user/query-language.md`, **Cuando** se revisan los type errors descritos, **Entonces**
       el caso `contains` con literal no-string sobre campo string queda declarado como
       comportamiento actual (`false` silencioso), con nota de que es candidato a alinearse con A-04
       → checklist estructural (revisión de diff).
     - **Criterio de aceptación (si se ratifica extenderlo a type error)**: **Dado** un campo string,
       **Cuando** se evalúa `campo contains 3` (literal numérico), **Entonces** la respuesta es
       `INVALID_SCHEMA`/`TypeError` en vez de `false` → **test: `contains_con_literal_no_string_sobre_string_es_type_error`**
       (solo se implementa si la ratificación llega antes de cerrar esta historia; si no, queda fuera).
     - ✅ **DESENLACE (2026-08-07)**: el usuario **ratificó el type error** antes de cerrar la
       historia, así que se ejecutó el **segundo** criterio, no el documental mínimo. Implementado en
       `crates/lodestar-core/src/eval.rs` con el test que esta spec nombra
       (`crates/lodestar-core/tests/consulta.rs`). El enunciado de arriba describe el estado
       **anterior** (`false` silencioso) — se conserva como redactado para no reescribir la historia.
       Sobre campo **lista** `contains` no cambia: sigue siendo pertenencia y admite literales de
       cualquier tipo, asimetría deliberada porque ahí el tipo del campo decide el significado.

  7. **Divergencia `workspace_status.counts` vs. `knowledge_check`**: el recuento que sirve
     `workspace_status` puede divergir del que computan `knowledge_check`/`graph_query` bajo ciertas
     condiciones de descubrimiento, descubierto al implementar `E29-H06`. Se documenta como
     **limitación conocida** en `contracts/mcp.yml` (sección de `workspace_status`): qué condición de
     descubrimiento puede producir la divergencia (a verificar y nombrar durante la implementación —
     la campaña no la especificó con detalle) y que el recuento con autoridad, en caso de duda, es el
     de `knowledge_check` (invariante #3: una sola verdad computada, y `workspace_status` es el
     resumen rápido, no el cómputo canónico).
     - **Criterio de aceptación**: **Dado** la condición de divergencia identificada, **Cuando** se
       reproduce con un workspace mínimo, **Entonces** el comportamiento (divergencia real u
       homogeneidad ya restaurada por trabajo posterior de E29) queda verificado y documentado con
       precisión — si la divergencia ya no existe tras E29-H06, el criterio pasa a ser "se confirma
       que no reproduce y se retira el seguimiento" → **test o revisión de diff, según lo que la
       verificación encuentre** (la historia no asume de antemano cuál de los dos desenlaces es
       cierto).

  8. **SARIF con `artifactLocation` bajo `.lodestar/`**: la salida SARIF de `check` puede listar
     rutas bajo `.lodestar/` en condiciones de borde tocadas por `E29-H01`/`E29-H06` (p. ej. el nuevo
     diagnóstico `WORKSPACE-EMPTY`, sin `target`, o un diagnóstico de descubrimiento anclado a la
     raíz). Se verifica el caso concreto: si el SARIF emitido incluye una ruta bajo `.lodestar/`
     (que no es un documento del workspace y no debería aparecer como `artifactLocation` de un
     hallazgo de contenido), se documenta como limitación conocida o se corrige con un guard de una
     línea si la causa es trivial (mismo criterio de "diagnóstico sin target" que ya resolvió
     `E29-H06` para `full_analysis`).
     - **Criterio de aceptación**: **Dado** un workspace con el diagnóstico `WORKSPACE-EMPTY` (o el
       caso de borde que la verificación confirme), **Cuando** se genera `lodestar check --sarif`,
       **Entonces** ninguna `artifactLocation` apunta bajo `.lodestar/` — o, si corregirlo excede el
       alcance de una línea, queda documentado como limitación conocida en `docs/user/ci.md` →
       **test: `sarif_no_lista_artefactos_bajo_lodestar`** (si se corrige) **o checklist
       estructural** (si se documenta).

  9. **`PATH-NOT-UTF8` sin red vía `full_analysis`**: el diagnóstico de path no-UTF8 puede alcanzar
     `full_analysis` sin cobertura de test dedicada — hueco detectado al endurecer el scope `paths`
     de `E29-H05`. Se cierra con un test de guardia que inyecte un `Check` con `PATH-NOT-UTF8` **sin
     `targets`** (el caso límite ya conocido por `E29-H06`: diagnósticos de descubrimiento sin target
     indexable) y verifique que `full_analysis` lo propaga en vez de descartarlo en silencio.
     - **Criterio de aceptación**: **Dado** un `Check` sintético con código `PATH-NOT-UTF8` y sin
       `targets`, **Cuando** se inyecta en el camino de `full_analysis`, **Entonces** aparece en el
       resultado final (`lodestar check` y `knowledge_check` scope `workspace`) → **test:
       `path_not_utf8_sin_targets_llega_a_full_analysis`**.

  10. **`protocolVersion` no-string**: `E29-H09` fijó el rechazo de una `protocolVersion`
      **soportada incorrectamente** (string mal formado o versión desconocida), pero el caso de un
      `protocolVersion` que directamente **no es un string** en el wire (número, objeto, `null`
      explícito) queda sin cubrir: `crates/lodestar-mcp/src/main.rs:249`,
      `params.get("protocolVersion").and_then(Value::as_str)` — si el valor no es string,
      `.and_then` devuelve `None` y el código cae al brazo de **ausente**, respondiendo con éxito y
      la versión por defecto, en vez de rechazar el tipo incorrecto. Se **tipa** (línea de código,
      no solo documentación, siguiendo la recomendación del enunciado): distinguir "la clave no está
      presente" de "la clave está presente con un tipo que no es string" antes de decidir la rama,
      y responder `-32602` (mismo código que la versión no soportada) cuando está presente pero no
      es string, nombrando el tipo recibido.
      - **Criterio de aceptación**: **Dado** un `initialize` con `protocolVersion: 123` (número),
        **Cuando** se procesa, **Entonces** la respuesta es un error JSON-RPC `-32602` que nombra
        que `protocolVersion` debe ser una cadena → **test:
        `protocol_version_no_string_es_rechazado`**.
      - **Dado** un `initialize` **sin** la clave `protocolVersion`, **Cuando** se procesa,
        **Entonces** sigue respondiendo con éxito y la versión por defecto, sin cambio → **test:
        `protocol_version_ausente_sigue_usando_el_default`** (control anti-vacuo: no se puede
        confundir "ausente" con "tipo incorrecto").

  11. **Mensaje duplicado de `INVALID_RESULT` del gate de staging**: el gate de staging que
      `E14-H04` introdujo emite el mismo mensaje por dos caminos que se componen. Causa raíz
      verificada: `WorkspaceError::InvalidResult` lleva la plantilla de `thiserror`
      `#[error("el resultado del plan no pasa la política de cambios: {0}")]`
      (`crates/lodestar-workspace/src/error.rs:28-29`), y el `String` que se le pasa en
      `crates/lodestar-workspace/src/staging.rs:265-271` **ya empieza con esa misma frase**
      (`format!("el resultado del plan no pasa la política de cambios: {nuevos} error(es)...")`) —
      el `Display` de `thiserror` la antepone otra vez, produciendo *"el resultado del plan no pasa
      la política de cambios: el resultado del plan no pasa la política de cambios: 2 error(es)..."*.
      **Arreglo de una línea**: el `format!` de `staging.rs:265` deja de repetir la frase que la
      variante de error ya antepone — pasa a construir solo la parte variable (`"{nuevos} error(es)
      nuevo(s), {} error(es) en total (rejectNewErrors={}, allowExistingErrors={})"`).
      - **Criterio de aceptación**: **Dado** un `change_apply` cuyo resultado no pasa la política de
        cambios, **Cuando** se lee el mensaje de error, **Entonces** la frase "el resultado del plan
        no pasa la política de cambios" aparece **una sola vez** → **test:
        `mensaje_de_invalid_result_no_esta_duplicado`**.

- **Fuera de alcance**: cualquier decisión de comportamiento no ratificada explícitamente (el caso
  6, `contains` con literal no-string, se documenta como mínimo y solo gana test de comportamiento
  si el usuario ratifica extenderlo durante esta historia); cualquier cambio en el catálogo de
  `ErrorCode` o de `CheckCode` (ninguno de los once puntos abre ni cierra un código); la
  implementación de recarga de config en caliente (A-09 se documenta, no se construye);
  ampliar el wire de `patch_frontmatter` para expresar "asignar `null`" (D-02 corrige la
  documentación en la dirección que **no** amplía el wire, tal como recomienda `decisiones §23`).

- **Delta de contrato** (`contracts/mcp.yml`): **parcial** — tocan el contrato los puntos A-01
  (semántica de `sections`), A-09 (ciclo de vida de la config), A-10 (redacción de
  `workspaceDirectory`) y el seguimiento 7 (limitación conocida de `workspace_status.counts`, si se
  confirma). Los puntos D-02 (ARCHITECTURE.md, no contrato), A-06 (docs/user/), el seguimiento 6
  (docs/user/, mínimo), el 8 (SARIF, según desenlace), el 9 (test interno, sin contrato) y el 10 y 11
  (código sin forma de contrato nueva, mismo catálogo) no lo tocan o lo tocan solo si el desenlace de
  su verificación lo exige.

- **Criterios de aceptación (agregados, además de los de cada punto)**:
  - **Dado** los once puntos de esta historia, **Cuando** se cierra, **Entonces** cada uno tiene su
    criterio saldado (test de guardia donde hay comportamiento a fijar, revisión de diff donde es
    prosa) — ningún punto queda "documentado a medias" sin que el checklist lo refleje →
    revisión de diff del propio documento de la historia frente al estado final del código.
  - **Dado** `docs/qa/campana-bugfixes-2026-08.md`, **Cuando** se actualiza al cerrar la épica,
    **Entonces** la fila «Fase 3 · D-02, A-01, A-06, A-09, A-10 · historia-escoba» pasa a cerrada, con
    los seis seguimientos nuevos marcados con su desenlace (documentado / corregido / convertido en
    historia futura) → checklist estructural.

- **Dependencias**: se beneficia de que `E30-H01` y `E30-H02` estén cerradas antes de redactar los
  puntos que las tocan de refilón (ninguno lo hace directamente, pero H01 cambia la redacción del
  parámetro `cursor` en el contrato, y sería redundante que H03 tocara la misma zona a la vez) — por
  eso el orden de construcción la deja al final, no por bloqueo técnico estricto.

- **Pruebas**: tests de guardia dispersos, cada uno en el crate que corresponde a su punto —
  `crates/lodestar-app/tests/` (A-06, seguimiento 7 si aplica), `crates/lodestar-core/tests/core.rs`
  (seguimiento 9, y el 6 si se ratifica extenderlo), `crates/lodestar-mcp/src/main.rs` `#[cfg(test)]`
  o `crates/lodestar-mcp/tests/mcp.rs` (seguimiento 10), `crates/lodestar-workspace/tests/` o
  `crates/lodestar-app/tests/` (seguimiento 11), `crates/lodestar-cli/tests/e2e.rs` (seguimiento 8,
  si se corrige). Fixtures: `lodestar-fixtures` donde el fixture mínimo ya cubra el caso; ninguno de
  los once puntos necesita un fixture nuevo dedicado.

- **Proceso**: ciclo **acotado** por punto (once micro-ciclos dentro de una sola historia): la
  mayoría son prosa o guards de una línea con riesgo bajo. El juez ciego verifica sobre todo que
  (a) el punto 6 no se extendió a comportamiento sin ratificación explícita, (b) D-02 corrigió las
  cinco citas exactas sin dejar ninguna con la redacción vieja, y (c) los criterios "según lo que la
  verificación encuentre" (7 y 8) llegaron a un desenlace declarado, no a un limbo.

---

## Orden de construcción

```
H01 (cursores estrictos)   ┐  independientes entre sí,
H02 (flakiness del lock)    ┘  paralelizables (paginación de lectura vs. concurrencia de publicación)

H03 (escoba documental y nits) al final: varios de sus once puntos citan de refilón zonas que H01
toca (semántica de «cursor» en contracts/mcp.yml) y su punto 6 (contains) es primo directo del A-04
que ya cerró E29-H04 — conviene que quien redacte la escoba vea el estado final de H01 antes de
escribir sobre paginación, aunque ningún punto de H03 dependa técnicamente de que H01/H02 estén
Done.
```

## Criterio de salida

Un cursor malformado o emitido por otra tool/consulta responde `INVALID_SCHEMA` nombrando el
problema, mientras que un cursor legítimo — obtenido de una respuesta real de la misma tool — sigue
paginando sin cambios, verificado con recorrido completo en las cuatro tools paginadas.
`crash_por_senal_no_deja_parciales` pasa 20 ejecuciones consecutivas de `cargo test --workspace`
sin fallar, con su causa raíz diagnosticada y documentada (no enmascarada), y deja de figurar en la
lista de flaky de la campaña. Los cinco nits originales de `§23` (D-02, A-01, A-06, A-09, A-10) y
los seis seguimientos de E28/E29 quedan saldados, cada uno con su criterio verificable — documental
donde el comportamiento ya era correcto, con test de guardia donde había un hueco real de cobertura,
o con el arreglo de una línea donde la campaña ya lo había localizado (el duplicado de
`INVALID_RESULT`). Ninguna de las tres historias introduce capacidad nueva de wire ni relitiga una
decisión ya tomada; `docs/qa/campana-bugfixes-2026-08.md` queda con las Fases 0-3 cerradas.
