# E34 — Interoperabilidad MCP por eras

> **Estado**: ratificada el **2026-08-17**. Esta épica traduce una arquitectura ya ratificada; no
> abre una nueva decisión de producto.

## Objetivo observable

`lodestar-mcp` es un servidor MCP por `stdio` que puede ser descubierto y consumido por clientes
oficiales de `rmcp` y por un arnés JSON-RPC crudo, hablando exactamente una de las dos eras
congeladas en cada request: **Modern MCP `2026-07-28`** (stateless, `server/discover`, metadata
por request) o **Legacy MCP `2025-11-25`** (`initialize`/`notifications/initialized`). Ambas eras
exponen la misma semántica de las diez tools, el mismo `App`, el mismo escritor transaccional y
el mismo catálogo único; ninguna respuesta válida depende del cliente, del transporte interno ni
de una cache accidental.

## Autoridades y referencias vigentes

- `ARCHITECTURE.md` §§2, 7.2, 19.6, 20.10, 20.11 y 20.14: MCP como fachada fina, stdio,
  diez tools, operaciones universales y único contrato de tipos.
- `docs/REFACTOR_PHASE_2.md`, fases 11–13: superficie MCP, transacciones y conservación del motor.
- `decisiones/03-transporte-mcp-rmcp.md`: deuda absorbida por esta migración; la decisión de
  adoptar `rmcp`, resolver la cancelación y conservar stdio queda ratificada por E34.
- `contracts/mcp.yml` y `contracts/README.md`: nombres, schemas, errores, perfiles y semántica
  vigentes de las tools; se actualizan con cada delta de esta épica.
- Especificación oficial [Modern MCP 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28)
  y [Legacy MCP 2025-11-25](https://modelcontextprotocol.io/specification/2025-11-25): solo como
  referencias normativas de wire de las dos eras congeladas.
- `crates/lodestar-app`, `crates/lodestar-mcp` y sus tests: contexto de la API existente. El
  prototipo histórico no es autoridad ni oráculo de comportamiento.
- `docs/qa/e34-interoperabilidad-mcp-evidencia.md`: hashes de locks, resultados de rojo/verde,
  gates y reproducción auditable de issue #38 conservados para el cierre.

## Congelados de la épica

- **Solo dos eras**: Modern `2026-07-28` y Legacy `2025-11-25`. No se aceptan fechas intermedias,
  futuras, draft ni aliases. Las fechas aparecen únicamente en `protocol_policy`; no se esparcen
  comparaciones de fecha por handlers, dispatcher o lógica de dominio.
- **Modern**: `server/discover` es descubrimiento opcional pero implementado; cada request lleva
  `_meta.io.modelcontextprotocol/protocolVersion` y
  `_meta.io.modelcontextprotocol/clientCapabilities` y
  `_meta.io.modelcontextprotocol/clientInfo`; no hay handshake ni sesión. Las respuestas
  exitosas llevan `resultType: "complete"`; los resultados cacheables (`server/discover` y
  `tools/list`) llevan `ttlMs` y `cacheScope` explícitos y conservadores.
- **Legacy**: `initialize` negocia `2025-11-25`; el cliente envía
  `notifications/initialized` sin respuesta; `ping` responde `result: {}`. Las respuestas no
  llevan `resultType`, `ttlMs` ni `cacheScope`.
- **Implementación**: `rmcp = 3.1.2` y Tokio viven únicamente en `lodestar-mcp`; MSRV `1.88` solo
  para esa crate. El resto del workspace conserva MSRV `1.80`. El único transporte de producto es
  stdio; no se añade HTTP ni OAuth.
- **Superficie**: las únicas tools son `workspace_status`, `knowledge_search`, `knowledge_get`,
  `metadata_inspect`, `knowledge_check`, `graph_query`, `impact_analyze`, `change_plan`,
  `change_apply` y `change_revert`. Catálogo y dispatcher son únicos; no hay tabla paralela por
  era, perfil o transporte. El `App` ejecuta las llamadas en serie.
- **No capacidades nuevas**: quedan fuera resources, prompts, sampling, elicitation, tasks/MRTR,
  subscriptions y cualquier extensión no enumerada; PR39 queda superada por esta arquitectura.

## Alcance

- Ratificar en código, contrato y documentación la política dual-era, sus versiones, capacidades,
  errores, metadata, cache hints, MSRV y dependencias.
- Extraer `LodestarMcpService` neutral al transporte, sobre el `App` existente, y probar paridad
  con la fachada estándar y `readonly`.
- Sustituir el bucle manual de líneas por `rmcp` sobre stdio, conservando stdout como stream MCP
  puro, stderr para logs y EOF como cierre limpio.
- Implementar las dos formas de lifecycle y el wire de `server/discover`, `tools/list`,
  `tools/call`, `ping`, notificaciones y errores, sin duplicar semántica de tools.
- Garantizar serialización de ejecución, cancelación segura y atomicidad transaccional.
- Actualizar `contracts/mcp.yml`, documentación de clientes y estado del proyecto; añadir harness
  crudo exacto y pruebas con clientes oficiales de `rmcp`.

## Fuera de alcance

- HTTP, SSE, WebSocket, OAuth, `Mcp-Session-Id`, autenticación, recursos, prompts, sampling,
  elicitation, tasks, MRTR, subscriptions, progress o streaming de resultados.
- Cambiar nombres, schemas o semántica de las diez tools, introducir DTOs paralelos o mover tipos
  fuera de `lodestar-core::types`/`lodestar-app`.
- Leer o escribir la cache como fuente de verdad, cambiar invariantes del workspace, o alterar el
  motor de transacciones fuera de la cancelación segura.
- Aceptar cualquier tercera era por compatibilidad, autodetectar fechas, o conservar el bucle
  manual como camino alternativo de producción.
- Implementar historias, cerrar decisiones adicionales o usar `prototype/index.html` como spec.

## Dependencias y orden

E34-H01 fija la política y las dependencias. E34-H02 depende de H01 y fija el servicio neutral.
E34-H03 depende de H02 y fija el transporte rmcp/stdio. E34-H04 y E34-H05 dependen de H03 y
cierran, respectivamente, Modern y Legacy. E34-H06 depende de H02–H05 y realiza la pasada de
serialización, cancelación, conformidad, documentación y cierre. No se implementa una historia
posterior si la anterior no está `Done`.

## Historias

### E34-H01 — Ratificar arquitectura, política dual-era y MSRV de la fachada

- **Objetivo**: dejar una política ejecutable y auditable que limite la fachada a las dos eras,
  `rmcp`/Tokio en `lodestar-mcp`, stdio y los diez nombres únicos.
- **Referencias**: `ARCHITECTURE.md` §§7.2, 20.10–20.11; `decisiones/03-transporte-mcp-rmcp.md`;
  `contracts/mcp.yml`; Modern/Legacy MCP enlazados arriba.
- **Alcance**:
  - Definir `protocol_policy` como única fuente de las fechas, metadata requerida, lifecycle y
    forma de respuesta por era.
  - Fijar `rmcp 3.1.2`, Tokio y MSRV 1.88 en `lodestar-mcp`; preservar MSRV 1.80 fuera de ella.
  - Declarar el catálogo de diez tools y las capacidades realmente ofrecidas, sin entradas
    reservadas que parezcan soportadas.
  - Registrar PR39 como superada y documentar los no objetivos de E34.
- **Fuera de alcance**: handlers, transporte, migración de código, o decidir una tercera versión.
- **Criterios de aceptación**:
  - **C1 — política única y dos eras**. **Dado** un request con una de las dos fechas congeladas,
    **cuando** se consulta `protocol_policy`, **entonces** devuelve exactamente la política de esa
    era y ningún otro módulo compara fechas. **Negativo**: una fecha antigua, futura, draft o
    intermedia se rechaza; `protocol_policy` no puede tener una lista de fallback implícita.
  - **C2 — dependencia y MSRV acotados**. **Dado** el workspace, **cuando** se inspeccionan
    `Cargo.toml`/metadata y se compila con MSRV, **entonces** solo `lodestar-mcp` declara
    `rmcp=3.1.2`, Tokio y `rust-version=1.88`, mientras las demás crates siguen en 1.80.
    **Negativo/anti-vacuidad**: una dependencia `rmcp` en otra crate o un MSRV elevado fuera de
    `lodestar-mcp` hace fallar la prueba.
  - **C3 — catálogo y capacidades cerrados**. **Dado** el registro resultante, **cuando** se
    enumeran tools y capacidades, **entonces** hay diez nombres únicos y solo `tools` en las
    capacidades anunciadas. **Negativo**: resources, prompts, sampling, elicitation, tasks,
    MRTR o subscriptions no aparecen ni son aceptados como métodos.
- **Pruebas**: `mcp_policy_matrix`, `mcp_dependency_scope_msrv`, `mcp_catalogo_unico_y_capacidades`
  (incluyen asserts de duplicados, fechas no soportadas y capacidades prohibidas).
- **Delta de contrato/docs**: `contracts/mcp.yml` gana la sección `protocol_policy`, las dos
  eras, MSRV, dependencias y lista cerrada; `docs/user/mcp-clients.md` deja de prometer la era
  histórica y documenta la política dual.

### E34-H02 — Extraer LodestarMcpService neutral y demostrar paridad

- **Objetivo**: concentrar lifecycle, catálogo, validación y dispatch en `LodestarMcpService`, sin
  que un transporte o perfil cree otra verdad.
- **Referencias**: `ARCHITECTURE.md` §§2, 7.2 y 20.10; `contracts/mcp.yml`; tests cross-fachada
  existentes.
- **Alcance**:
  - Servicio neutral que recibe el `App` y `Profile`, expone discover/list/call/ping y devuelve
    resultados MCP estructurados sin leer stdin/stdout.
  - Un catálogo y un dispatcher; `standard` incluye las diez tools y `readonly` oculta únicamente
    las tres de cambio.
  - Delegación a `App`/core para toda semántica y preservación del envelope, códigos y schemas.
- **Fuera de alcance**: rmcp, framing, concurrencia, cancelación y cambios de negocio.
- **Criterios de aceptación**:
  - **C1 — paridad de catálogo y llamada**. **Dado** el mismo `App`, perfil y request válido,
    **cuando** se invoca el servicio neutral y la fachada estándar, **entonces** `tools/list` y
    `tools/call` son byte/semánticamente equivalentes (salvo framing permitido). **Negativo**:
    una llamada desconocida, argumento inválido o error de dominio conserva el mismo código y no
    se convierte en éxito vacío.
  - **C2 — paridad standard/readonly**. **Dado** un workspace fixture, **cuando** se consulta
    cada tool en ambos perfiles, **entonces** las siete lecturas/verificaciones tienen el mismo
    resultado y las tres de cambio solo existen en `standard`. **Guard anti-vacuidad**: se ejecuta
    una lectura no vacía y una escritura real; no basta comparar dos listas vacías.
  - **C3 — una sola verdad**. **Dado** el catálogo y dispatcher del servicio, **cuando** se
    inspecciona el registro, **entonces** cada nombre aparece una vez y cada llamada pasa por el
    mismo handler independientemente de era/perfil. **Negativo**: un alias legacy o una tabla por
    transporte falla la prueba estructural.
- **Pruebas**: `service_paridad_standard_readonly`, `service_golden_cross_fachada_no_vacia`,
  `service_catalogo_dispatcher_sin_duplicados`.
- **Delta de contrato/docs**: el contrato identifica `LodestarMcpService` como dueño de catálogo y
  semántica; la documentación de integración describe perfiles como filtro del mismo servicio,
  no como servidores distintos.

### E34-H03 — Sustituir el bucle manual por rmcp sobre stdio

- **Objetivo**: servir el servicio neutral a través de `rmcp 3.1.2` por stdio, con framing correcto,
  stdout limpio, stderr para logs y ejecución serial.
- **Referencias**: `ARCHITECTURE.md` §7.2; `decisiones/03-transporte-mcp-rmcp.md`; Modern/Legacy
  transports; restricciones de esta épica.
- **Alcance**:
  - Arranque del binario y adaptador rmcp/stdio, sin segundo bucle manual de producción.
  - Conservar argumentos `--root`/`--profile`, EOF limpio, errores JSON-RPC y flush de respuestas.
  - Integrar un executor serial del `App`; no ejecutar dos llamadas sobre el mismo estado a la vez.
  - Harness raw que pueda enviar bytes exactos y capturar separadamente stdout/stderr.
- **Fuera de alcance**: negociación semántica completa de cada era (H04/H05), HTTP y streaming.
- **Criterios de aceptación**:
  - **C1 — stdout/logs**. **Dado** un proceso con logs de arranque y una secuencia válida,
    **cuando** se captura stdout y stderr por separado, **entonces** stdout contiene únicamente
    mensajes JSON-RPC válidos, uno por respuesta, y los logs están solo en stderr. **Negativo**:
    cualquier banner, ANSI o log en stdout falla; un JSON parcial no cuenta como respuesta.
  - **C2 — EOF y framing**. **Dado** un stdin que termina después de una o más requests,
    **cuando** llega EOF, **entonces** el proceso termina limpiamente sin panic, thread huérfano ni
    respuesta inventada. **Negativo**: línea vacía, JSON inválido y request sin id siguen las
    reglas del contrato (error o notificación sin respuesta), no bloquean el proceso.
  - **C3 — serialización observable**. **Dado** dos llamadas concurrentes sobre un fixture con una
    barrera, **cuando** se envían al servidor, **entonces** el executor serial las completa en un
    orden válido y nunca hay carreras, doble escritor ni respuestas cruzadas. **Guard
    anti-vacuidad**: una llamada debe observar el efecto de la otra y el test falla si ambas solo
    devuelven lecturas constantes.
  - **C4 — clientes oficiales**. **Dado** un cliente oficial compatible con `rmcp`, **cuando** se
    conecta a stdio, **entonces** puede completar discovery/initialize según la era seleccionada y
    listar una tool real. **Negativo**: el proceso no requiere un protocolo privado ni una línea de
    saludo adicional.
- **Pruebas**: `stdio_stdout_puro_y_logs_en_stderr`, `stdio_eof_limpio`,
  `stdio_concurrencia_executor_serial`, `rmcp_official_discovery_initialize` y
  `harness_raw_frames_exactos`.
- **Delta de contrato/docs**: `contracts/mcp.yml` sustituye el bucle manual por rmcp/stdio como
  transporte vigente y fija las reglas de framing, EOF y logs; `docs/user/mcp-clients.md` añade
  comandos de conexión oficial y harness raw.

### E34-H04 — Completar contrato Modern MCP 2026-07-28

- **Objetivo**: hacer conformes las requests modernas stateless y las respuestas de discovery,
  listado y llamada, sin introducir lifecycle legacy en la era moderna.
- **Referencias**: Modern MCP 2026-07-28; `ARCHITECTURE.md` §§7.2/20.10; `contracts/mcp.yml`.
- **Alcance**:
  - `server/discover` con versiones soportadas, capacidades y metadata del servidor.
  - Validación de `_meta.io.modelcontextprotocol/protocolVersion`,
    `_meta.io.modelcontextprotocol/clientCapabilities` y
    `_meta.io.modelcontextprotocol/clientInfo` por request; la identidad contiene al menos `name`
    y `version` string no vacíos.
  - `tools/list` y `tools/call` con el catálogo único, `resultType: "complete"` y hints de cache
    donde la era los exige; errores modernos `-32602` para metadata ausente y `-32022` para versión
    no soportada.
  - `ping` moderno no forma parte del catálogo de methods implementados y devuelve method not found.
  - La metadata viaja exactamente en `params._meta`; `serverInfo` se publica en la clave reservada
    `io.modelcontextprotocol/serverInfo` de la metadata de respuesta, nunca como campo ad hoc.
- **Fuera de alcance**: initialize/initialized, tasks/MRTR, resources/prompts y HTTP.
- **Criterios de aceptación**:
  - **C1 — discover moderno**. **Dado** un request `server/discover` con metadata completa y
    versión `2026-07-28`, **cuando** se procesa, **entonces** responde con `supportedVersions` que
    contiene exactamente `2026-07-28`, capabilities solo de tools e instrucciones, `resultType:
    "complete"`, `ttlMs`, `cacheScope` y `serverInfo` tipado con `name` y `version` string no
    vacíos. **Guard**: como `capabilities.tools` no enumera nombres,
    el test encadena un `tools/list` Modern y exige las diez tools reales; rechaza un discover que
    pase solo por devolver un objeto vacío.
  - **C2 — metadata faltante y -32022**. **Dado** un request moderno sin
    `io.modelcontextprotocol/protocolVersion`, sin `clientCapabilities` o sin `clientInfo`,
    **cuando** llega al
    servidor, **entonces** responde `-32602` (Invalid params) y no ejecuta la tool. **Dado** un
    request con una fecha moderna no soportada, **cuando** llega al servidor, **entonces** responde
    `-32022` con `data.requested` igual a la fecha enviada y `data.supported` igual a la lista
    Modern declarada, y no ejecuta la tool. La prueba abre antes el lifecycle stateless con un
    discover válido para poder observar el `-32602` por petición de rmcp. **Negativo**: metadata
    fuera de `params._meta`, tipos `null`/no string, capacidades nulas o identidad sin `name` y
    `version` string no vacíos tampoco valen; no hace fallback a Legacy, no usa una versión por
    defecto y no devuelve `result`. La guarda se ejerce en discovery, list y call, no sólo en una
    clase de request.
  - **C3 — list/call modernos**. **Dado** metadata moderna válida en cada request, **cuando** se solicitan
    `tools/list` y una `tools/call` de lectura y otra de cambio, **entonces** devuelve los diez
    schemas vigentes comparados con el catálogo neutral/canónico, no con otra era servida por el
    mismo wire, y resultados de la misma semántica de `App`. **Negativo**: `initialize`,
    `notifications/initialized`, `resources/list` y una tool desconocida no ejecutan nada; cada
    rechazo conserva su código JSON-RPC.
  - **C4 — `resultType` y cache**. **Dado** una respuesta moderna exitosa de `server/discover` o
    `tools/list`, **cuando** se deserializa con los tipos oficiales de rmcp 3.1.2, **entonces** contiene
    `resultType: "complete"`, `ttlMs` entero no negativo y `cacheScope` válido; una respuesta
    moderna de `tools/call` también contiene `resultType`. **Negativo/anti-vacuidad**: el mismo
    frame sin cualquiera de esos campos falla el test moderno; el valor no puede ser una cache
    mutable del workspace ni variar por una llamada equivalente.
  - **C5 — ping moderno**. **Dado** `ping` con metadata moderna, **cuando** se procesa, **entonces**
    responde error `-32601` method not found. **Negativo**: no devuelve `{}` ni se confunde con el
    ping legacy.
- **Pruebas**: `modern_discover`, `modern_metadata_faltante_es_32602_y_version_es_32022`,
  `modern_tools_list_y_call`, `modern_result_type_y_cache_hints`, `modern_ping_method_not_found`,
  `modern_schema_oficial` y `harness_raw_modern_exacto`.
- **Delta de contrato/docs**: `contracts/mcp.yml` añade schemas y errores Modern, `server/discover`,
  metadata obligatoria, `resultType` y cache hints; `docs/user/mcp-clients.md` documenta el flujo
  stateless y la prohibición de initialize en esta era.

### E34-H05 — Completar baseline Legacy MCP 2025-11-25

- **Objetivo**: mantener interoperabilidad explícita con clientes Legacy mediante handshake,
  notificación `initialized` y `ping`, sin filtrar campos Modern a sus respuestas y sin crear una
  tabla de compatibilidad histórica.
- **Referencias**: Legacy MCP 2025-11-25 lifecycle/tools/schema; `ARCHITECTURE.md` §7.2;
  `contracts/mcp.yml` (`protocol_policy.Legacy`); `decisiones/03-transporte-mcp-rmcp.md` §3
  (antecedente `issue #38`, `PR #39` superada). El transcript de regresión de issue #38 es el
  comando raw exacto indicado en C5; no se toma el prototipo como oráculo.
- **Dependencias**: E34-H01, H02, H03 y H04 en estado `Done`; consume el `LodestarMcpService`,
  el catálogo/dispatcher único y el transporte `rmcp`/stdio ya establecidos.
- **Alcance**:
  - `initialize` con respuesta negociada exactamente a `2025-11-25`, `serverInfo`, capability
    `tools` e instrucciones derivadas del perfil activo; `notifications/initialized` es una
    notificación sin respuesta.
  - Probar que cualquier revisión **string** ofrecida en un `initialize` bien formado —baseline,
    revisiones históricas, Modern, futura o inventada— selecciona siempre `2025-11-25`; la cadena
    solicitada no se ecoa, no selecciona Modern y no crea una tercera era. H05 sólo clasifica
    revisiones string dentro de un `initialize` bien formado.
  - `ping` Legacy con respuesta `result: {}` y `tools/list`/`tools/call` sobre el catálogo neutral,
    conservando exactamente orden, descripción, schemas, perfiles y semántica de `App`.
  - Ausencia explícita de `resultType`, `ttlMs` y `cacheScope` en los campos reservados del
    envelope/result Legacy.
  - Reproducir de forma auditable el fallo de issue #38 y dejar evidencia de que PR #39 queda
    superada: el arreglo no es ampliar una lista de revisiones, sino negociar una única baseline.
- **Fuera de alcance**: `server/discover` Modern, metadata Modern obligatoria, versiones
  históricas como eras soportadas, tasks/MRTR, resources, prompts y transports de red.
- **Criterios de aceptación**:
  - **C1 — initialize y negociación cerrada**. **Dado** un `initialize` MCP bien formado con
    `protocolVersion` en la matriz `{2025-11-25, 2024-11-05, 2025-03-26, 2025-06-18,
    2026-07-28, 2099-12-31, 1990-01-01}`, **cuando** se procesa cada caso, **entonces** el
    primer resultado correlacionado responde `protocolVersion: "2025-11-25"`, `serverInfo` con
    `name`/`version` no vacíos, capability `tools` e `instructions` coherentes con el perfil; la
    respuesta no contiene un error ni la fecha ofrecida. **Guardas anti-vacuidad**: se comprueban
    baseline, histórica, Modern y futura, no solo un valor conocido; se verifica que la respuesta
    de `initialize` precede cualquier request posterior y que ningún cambio en el workspace ocurre
    durante la negociación.
  - **C2 — `notifications/initialized` es silenciosa**. **Dado** un `initialize` exitoso, **cuando**
    el cliente envía exactamente `{"jsonrpc":"2.0","method":"notifications/initialized"}` sin
    `id`, **entonces** stdout no contiene respuesta para esa línea, el proceso sigue vivo y la
    siguiente request Legacy responde con su mismo id. **Negativo**: el mismo método enviado con un
    `id` deja de ser una notificación y recibe un error correlacionado (método no encontrado,
    `-32601`, según el dispatcher rmcp); no se descarta silenciosamente ni se confunde con una
    respuesta de `initialize`. La prueba distingue silencio de proceso muerto mediante una request
    posterior y EOF limpio.
  - **C3 — ping Legacy**. **Dado** una sesión que ya negoció `2025-11-25`, **cuando** se envía
    `ping` con un id único, **entonces** responde exactamente ese id con `result: {}`. **Negativos**:
    no devuelve `-32022`, `-32601` ni un error Modern, no incluye `resultType`/hints Modern y no
    ejecuta ni altera una tool o el workspace.
  - **C4 — catálogo único y envelope Legacy**. **Dado** el handshake seguido de `initialized`,
    **cuando** se solicitan `tools/list` y `tools/call` en `standard` y `readonly`, **entonces**
    `tools/list` conserva el orden y los diez (standard) o siete (readonly) nombres, descripciones,
    `inputSchema` y `outputSchema` del catálogo canónico; una lectura real y una escritura real
    conservan la semántica, `structuredContent`, `isError` y códigos de dominio de `App`, y
    readonly oculta/rechaza únicamente las tres tools de cambio. `server/discover` no pertenece a
    una sesión Legacy y responde `-32601`, sin filtrar un resultado Modern. **Guard anti-vacuidad**: se
    ejecutan al menos una lectura no vacía, una escritura real y un rechazo de escritura en
    readonly. **Además**, cada respuesta Legacy exitosa de `initialize`, `tools/list`, `tools/call`
    y `ping` carece de `resultType`, `ttlMs` y `cacheScope` en los campos reservados del envelope;
    la ausencia se comprueba estructuralmente, no buscando un substring. **Negativo**: no se acepta
    un segundo registro Legacy, una lista histórica, una respuesta Modern disfrazada ni un éxito
    vacío para una tool desconocida o no permitida; ambos rechazos se ejercen por el wire Legacy.
  - **C5 — reproducción exacta de issue #38 y cierre de PR #39**. **Dado** un fixture mínimo de
    workspace y el proceso arrancado desde ese directorio con el comando exacto
    `printf '%s\\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}}}' | lodestar-mcp --profile standard`, **cuando** stdin termina por EOF después de ese único
    frame, **entonces** el binario real termina sin hang y stdout contiene exactamente un frame
    JSON-RPC con `id: 1`, `result.protocolVersion: "2025-11-25"`, capability `tools`,
    `serverInfo`, `instructions` y ningún `error`; stderr puede contener el diagnóstico de arranque,
    pero stdout no contiene banner, logs, ANSI ni líneas adicionales. El harness compara el frame
    completo (bytes y newline), ids, orden, cierre por EOF y fixture; no acepta `contains`, regex o
    un substring. **Guardas**: falla si reaparece el `-32602` que mencionaba la lista histórica
    `2024-11-05/2025-03-26/2025-06-18`, si se ecoa otra revisión, si se responde Modern o si el
    proceso muere antes de escribir el resultado. La matriz C1 demuestra además que añadir
    `2025-11-25` a esa lista (la solución parcial de PR #39) no es la solución E34: una revisión
    antigua o futura sigue negociando la misma baseline sin otra rama.
- **Pruebas**: `legacy_initialize_y_negociacion_fechas` (C1),
  `legacy_initialized_sin_respuesta` (C2), `legacy_ping` (C3),
  `legacy_tools_list_call` + `legacy_sin_result_type_ni_cache_hints` (C4),
  `issue_38_repro_exacta` + `harness_raw_legacy_exacto` (C5). Las pruebas arrancan el binario
  real y separan stdout/stderr; los tests de cliente oficial Legacy quedan incluidos en H06.
- **Delta de contrato/docs**: `contracts/mcp.yml` completa `protocol_policy.Legacy` con
  `initialize`, `notifications/initialized`, `ping`, `tools/list` y `tools/call`, la negociación
  incondicional de `2025-11-25`, la forma de `serverInfo`/`instructions` y la prohibición de
  `resultType`/`ttlMs`/`cacheScope`; `docs/user/mcp-clients.md` mantiene el ejemplo completo de
  `initialize`/`initialized`, documenta la matriz de negociación y enlaza la reproducción de
  issue #38, dejando explícito que PR #39 queda superada. `IMPLEMENTATION_STATUS.md` y §3 de
  `decisiones/03-transporte-mcp-rmcp.md` se actualizan al cerrar la historia.

### E34-H06 — Serialización, cancelación segura, conformidad, docs y cierre

- **Objetivo**: cerrar la entrega con ejecución serial observable, cancelación cooperativa sin
  transacciones parciales, conformidad comprobada con rmcp 3.1.2 y documentación/gates reproducibles.
- **Referencias**: `ARCHITECTURE.md` §§2, 7.2, 20.11; `decisiones/03-transporte-mcp-rmcp.md`;
  `contracts/mcp.yml`; y el contrato/documentación actualizados en E34-H01–H05.
  En rmcp 3.1.2, `notifications/cancelled` lleva `params.requestId` opcional y razón, cancela el
  `RequestContext.ct` de la request asociada y no tiene respuesta. La cancelación es cooperativa:
  rmcp retira la respuesta pendiente, pero no interrumpe una función síncrona ni deshace efectos
  de disco. H06 debe respetar esa semántica y no introducir tasks, MRTR ni otro mecanismo de
  cancelación.
- **Alcance**:
  - Comprobar `RequestContext.ct` en los límites seguros del adaptador: una request cancelada
    mientras espera el turno serial no entra en `LodestarMcpService` ni ejecuta una tool; si ya
    tomó el turno o cruzó el punto de publicación, la operación termina mediante la transacción
    existente y no se intenta despublicarla. La respuesta de una request efectivamente cancelada
    puede ser suprimida por rmcp; el test no puede exigir un frame de error inventado.
  - Repetir el banco por las eras Modern/Legacy y los perfiles `standard`/`readonly`; validar
    schemas, errores, stdout/stderr, EOF, transcript raw exacto y clientes oficiales rmcp.
  - Cerrar instalación/uso, compatibilidad de eras, troubleshooting, MSRV y pins de dependencias;
    dejar contrato, estado, notas de migración y gates alineados.
- **Fuera de alcance**: tareas durables, reanudación, cancelación HTTP, `resources`, `prompts`,
  `sampling`, `elicitation`, `tasks`, MRTR, subscriptions, paralelismo de tools o nuevas eras.
- **Criterios de aceptación**:
  - **C1 — cancelación cooperativa y transacción indivisible**. **Dado** un fixture que mantiene
    una `tools/call` de escritura detrás de un punto de sincronización de tests, antes de tomar el
    turno serial, **cuando** el harness raw envía
    `notifications/cancelled` con el `requestId` de esa llamada y libera el punto, **entonces** rmcp
    no emite respuesta para ese id, la tool no se ejecuta, los bytes de todos los `.md` permanecen
    iguales, no aparece receipt aplicable ni journal pendiente, y una request posterior válida
    completa. El punto de sincronización debe ser determinista (canal/barrera o seam gateado para
    tests), nunca un `sleep` o una carrera temporal. **Negativo**: una cancelación sin
    `requestId` no cancela otra request y una cancelación tardía, después del primer rename, no
    restaura ni borra una publicación; el receipt y el estado de recuperación siguen siendo válidos
    y el canónico queda completo, nunca a mitad.
  - **C2 — exclusión serial bajo carga**. **Dado** el mismo `SerialExecutor<LodestarMcpService>`
    y llamadas superpuestas de lectura, planificación y publicación, **cuando** se envían desde
    cuatro combinaciones independientes (Modern/Legacy × `standard`/`readonly`), **entonces** cada
    respuesta conserva su id y JSON completo, ninguna operación observa un `App` a mitad de otra,
    las revisiones y bytes finales coinciden con algún orden serial observable y la única
    `WRITE_CONFLICT` que aparezca es la causada por un plan realmente obsoleto. **Guard
    anti-vacuidad**: el fixture debe bloquear una llamada con una barrera de tests (no con tiempo
    dormido), demostrar que la segunda no entra antes de liberar la primera, inspeccionar los bytes
    finales y ejecutar después dos planes nuevos válidos consecutivos para descartar un rechazo
    posicional o el rechazo de toda concurrencia.
  - **C3 — clientes oficiales rmcp y harness raw**. **Dado** un cliente auxiliar que dependa
    literalmente de `rmcp = 3.1.2`, use `ClientLifecycleMode::Discover` con solo
    `ProtocolVersion::V_2026_07_28`, y otro que use `ClientLifecycleMode::Initialize` con la
    baseline Legacy, **cuando** el primero hace `server/discover`, `tools/list` y `tools/call` y el
    segundo `initialize`, `notifications/initialized`, `ping`, `tools/list` y `tools/call`,
    **entonces** ambos deserializan los tipos oficiales, reciben el catálogo exacto filtrado por
    perfil y la semántica de su era, y cierran por EOF sin dejar tareas o procesos vivos. El
    harness raw complementario compara bytes completos (incluido LF), ids, orden y ausencia de
    respuesta para notificaciones. **Negativos**: metadata Modern ausente/malformada, versión
    Modern no soportada, `server/discover` en Legacy, `ping` en Modern y tool desconocida deben
    devolver exactamente el error de contrato; ninguna prueba puede aprobar por `contains`, por
    aceptar un esquema vacío o por omitir el auxiliar oficial si falta una dependencia.
  - **C4 — stdout, stderr y EOF**. **Dado** el binario real en cada era y perfil, **cuando** se
    capturan los tres descriptores por separado y se cierra stdin después de una secuencia válida,
    **entonces** stdout contiene únicamente frames JSON-RPC completos delimitados por LF, stderr
    contiene los diagnósticos de arranque/ejecución, y EOF produce terminación limpia y status
    exitoso. **Negativo**: banner, ANSI, log, panic, bytes parciales o una respuesta para
    `notifications/initialized`/`notifications/cancelled` en stdout falla; el harness compara el
    transcript exacto, no una subcadena.
  - **C5 — documentación, MSRV, pins y gates**. **Dado** el diff completo contra `develop`,
    **cuando** se ejecutan `scripts/agent-gates.sh contract`, `scripts/agent-gates.sh policy`,
    `scripts/agent-gates.sh full`, `cargo metadata --locked`, y los checks con Rust 1.80 para el
    workspace sin MCP y Rust 1.88 para `lodestar-mcp`, **entonces** el contrato y docs describen
    solamente Modern `2026-07-28` y Legacy `2025-11-25`, el catálogo y dispatcher siguen teniendo
    una única fuente, rmcp/Tokio no contaminan crates ajenas, los pins aprobados constan en
    `Cargo.toml`/`Cargo.lock`, y todos los gates pasan. **Guard anti-vacuidad**: los checks fallan
    ante `ProtocolVersion::KNOWN_VERSIONS`, una fecha histórica/futura fuera de
    `protocol_policy`, una referencia activa al bucle manual o a la PR #39 como solución, una
    capacidad excluida o un ejemplo no ejecutable de cualquiera de las dos eras.
- **Pruebas**: `cancelacion_transaccional_sin_parciales`,
  `serializacion_concurrente_final_coherente`, `rmcp_clientes_oficiales_moderno_y_legacy`,
  `harness_raw_exacto_completo`, `stdout_stderr_eof_final`,
  `cargo test --workspace`, `cargo metadata --locked`,
  `scripts/agent-gates.sh contract`, `scripts/agent-gates.sh policy` y
  `scripts/agent-gates.sh full`. Los fixtures deben incluir al menos un documento enlazado, un
  plan de escritura válido, un perfil `readonly`, y un seam de sincronización gateado para ejercer
  cancelación/serialización sin sleeps.
- **Delta de contrato/docs**: revisión final de `contracts/mcp.yml`, `ARCHITECTURE.md`,
  `decisiones/03-transporte-mcp-rmcp.md`, `docs/user/mcp-clients.md`, notas de migración,
  `IMPLEMENTATION_STATUS.md` y changelog; se conserva la reproducción de issue #38 y se registra
  explícitamente que PR #39 queda superada por E34. El contrato debe documentar que la cancelación
  stdio es la notificación rmcp cooperativa, que una cancelación tardía no revierte una transacción
  ya publicada, y que E34 queda cerrada con stdio/rmcp y exactamente esas dos eras.

## Definición de Done de E34

Las seis historias están `Done` solo cuando sus pruebas observables pasan con el binario real,
`cargo clippy --workspace --all-targets -- -D warnings` no introduce warnings, el contrato y la
documentación coinciden con el wire, los tests raw y clientes oficiales cubren ambas eras, y los
invariantes de Lodestar (Markdown como verdad, core puro, una sola verdad computada, un contrato de
tipos, único escritor y `RelPath`) siguen verificados. La épica no se implementa ni se da por
cerrada por el mero hecho de ratificar este texto.

## Texto de ratificación

> Ratifico E34 — Interoperabilidad MCP por eras — el **2026-08-17** con las seis historias y el
> orden H01→H06 anteriores. Congelo Modern MCP `2026-07-28` y Legacy MCP `2025-11-25`, fechas solo
> en `protocol_policy`; `rmcp 3.1.2`/Tokio únicamente en `lodestar-mcp`; MSRV 1.88 solo para esa
> crate y 1.80 para el resto; solo stdio; catálogo/dispatcher únicos de diez tools; executor
> serial del `App`; sin HTTP/OAuth/resources/prompts/sampling/elicitation/tasks/MRTR/subscriptions;
> PR39 superada. Autorizo pasar a `$ciclo` por historia cuando sus dependencias estén `Done`.
