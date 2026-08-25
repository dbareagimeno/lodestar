---
id: E35-H01
titulo: "Contrato de memoria retenida y configuración performance.maxMemory"
estado: "ratificada"
ratificada_en: "2026-08-24"
origen: "GitHub issue #53, titulada originalmente [E34-H01]"
trazabilidad: "La colisión histórica E34-H01 se resuelve materializando esta historia como E35-H01"
---

# E35-H01 — Contrato de memoria retenida y configuración `performance.maxMemory`

## Objetivo observable

Al abrir un workspace, Lodestar debe poder cargar una única configuración pública de presupuesto
(`performance.maxMemory`) y construir una única contabilidad determinista `MemoryBudget` en bytes.
El presupuesto debe aceptar solo la gramática y el rango ratificados, producir errores accionables
por el camino existente y particionar exactamente `N` bytes entre SQLite, W-TinyLFU y trabajo. La
historia conserva el estado normativo **ratificada** y su alcance está implementado.

La issue #53 conserva en GitHub el título original `[E34-H01] Contrato de memoria y configuración
performance.maxMemory`. Ese ID colisiona con la historia ya cerrada de interoperabilidad MCP; la
trazabilidad local normativa es **E34-H01 → E35-H01**. Esta historia no reabre ni renombra E34.

## Autoridades y referencias vigentes

- `ARCHITECTURE.md §23` — contrato de configuración, ownership, contabilidad y redondeo.
- `docs/REFACTOR_PHASE_2.md`, adenda E35-H01 — comportamiento de configuración de la migración.
- `docs/SCALABILITY_ANALYSIS.md` — memoria retenida/controlable frente a RSS y evidencia diagnóstica.
- `decisiones/14-store-sin-consumidor.md` y `decisiones/README.md` — §14 permanece abierta; esta
  historia no elige conectar, acotar ni retirar el store.
- `IMPLEMENTATION_STATUS.md` — estado real de entrega de la historia: implementada; el frontmatter
  de esta spec conserva el estado normativo ratificada.
- `crates/lodestar-workspace/src/config.rs`, `src/lib.rs` y `src/error.rs` — chokepoints existentes
  de carga, `Workspace::open` y error de configuración; son contexto de integración, no una spec
  alternativa.
- GitHub issue [#53](https://github.com/dbareagimeno/lodestar/issues/53) — origen y trazabilidad.

El prototipo histórico no es referencia normativa y no se usa para diseñar ni verificar esta historia.

## Alcance

- Añadir la sección opcional `performance.maxMemory` a la configuración vigente.
- Usar default `256MiB`, mínimo `64MiB` y gramática exacta `[1-9][0-9]*(MiB|GiB)` case-sensitive
  sobre el scalar YAML semántico deserializado.
- Convertir MiB/GiB a bytes `u64` con factores binarios y operaciones/conversiones *checked*.
- Definir `MemoryBudget` y sus tres subpresupuestos, con construcción única al abrir el workspace.
- Fijar mensajes accionables y las pruebas de parsing, rango, overflow, ownership y partición.
- Actualizar la documentación y el estado sin modificar el contrato MCP.

## Fuera de alcance

- Conectar SQLite al camino de lectura o activar un consumidor del store.
- Implementar W-TinyLFU, su política de admisión/evicción o cualquier runtime de cache.
- Ejecutar la semántica fuera de cache, el fallo por documento que no quepa o la protección contra
  *thrashing*: solo se documenta y se difiere a las historias posteriores **#55, #57, #59 y #62**.
- Medir o limitar RSS, cgroup, allocator, mmap/OS o memoria temporal inevitable.
- Autotuning, presets, knobs de SQLite/W-TinyLFU, cuarta reserva o límites MCP nuevos.
- Cambiar `contracts/mcp.yml`, añadir códigos/campos MCP o resolver `decisiones §14`.

## Contrato de configuración y contabilidad

La única perilla pública es:

```yaml
performance:
  maxMemory: 256MiB
```

Si se omite `performance.maxMemory`, el valor efectivo es `256MiB`. Son válidos, entre otros,
`64MiB`, `256MiB` y `2GiB`. La cadena debe satisfacer exactamente
`[1-9][0-9]*(MiB|GiB)` **sobre el scalar YAML semántico ya deserializado**, no sobre los
caracteres de formato fuente. Por ello, el whitespace sintáctico separado antes de un newline,
comentario, coma o `}` no forma parte del valor y es válido: `performance: {maxMemory: 256MiB }`
equivale a `256MiB`. Sí son inválidos los scalars cuyo contenido incluye espacios, como
`"256MiB "`, `" 256MiB"` o `256 MiB`; tampoco se aceptan fracciones, ceros iniciales,
`MB`/`GB`, `mib`/`gib` ni otras unidades. No se introduce un scanner/lexer source-aware ni un
parser YAML paralelo. La conversión usa `1024^2` y `1024^3`, no factores decimales.

Sea `N` el valor convertido en bytes. La partición es, exactamente y en este orden:

```text
SQLite     = floor(30 * N / 100)
W-TinyLFU  = floor(20 * N / 100)
Work       = N - SQLite - W-TinyLFU
```

Todo residuo de las dos divisiones va a `Work`; por eso `SQLite + W-TinyLFU + Work = N` para todo
`N`, y `Work` queda protegido frente a las caches. `Work` es al menos el 50 % de `N`, pero no se
calcula como `floor(50 * N / 100)`. No existe una cuarta reserva ni un pool sin tope.

`MemoryBudget` se construye una sola vez por `lodestar-workspace::Workspace::open`. Ese es su único
owner; core, store y fachadas no crean ni poseen presupuestos adicionales.

## Criterios BDD binarios y pruebas propuestas

Cada criterio tiene una observación positiva y una guarda explícita contra una implementación vacua.

### C1 — Default y valores válidos

**Dado** un workspace sin `performance.maxMemory`, o con `64MiB`, `256MiB` o `2GiB`;
**Cuando** se carga la configuración y se solicita el valor en bytes;
**Entonces** la carga tiene éxito y devuelve, respectivamente, `256 * 1024^2`, `64 * 1024^2`,
`256 * 1024^2` y `2 * 1024^3` bytes.

Prueba propuesta: tabla de integración en `crates/lodestar-workspace/tests/config.rs` que cubra
ausencia y los tres valores, incluyendo el valor efectivo observado. Guarda anti-vacuidad: comprobar
el número en bytes impide que el parser solo valide la forma o devuelva siempre el default.

### C2 — Gramática estricta sobre el scalar YAML

**Dado** estas cuatro configuraciones válidas, donde el whitespace es sintáctico y no parte del
scalar:

```yaml
performance:
  maxMemory: 256MiB
```

```yaml
performance:
  maxMemory: 256MiB # comentario
```

```yaml
performance: {maxMemory: 256MiB }
```

```yaml
performance: {maxMemory: 256MiB ,}
```

y además scalars cuyo contenido es `0MiB`, `064MiB`, `1.5GiB`, `"256MiB "`, `" 256MiB"`,
`256 MiB`, `256mib` o `256MB`;
**Cuando** se deserializa el YAML y se carga la configuración;
**Entonces** cada forma sintáctica válida tiene éxito y produce el mismo valor semántico `256MiB`,
mientras cada scalar inválido falla con un error que nombra
`performance.maxMemory`, el valor recibido y la regla de gramática incumplida.

La validación se realiza únicamente sobre el scalar YAML semántico: el whitespace sintáctico antes
de newline, comentario, coma o `}` no integra el valor. No se inspecciona el texto fuente con un
scanner/lexer adicional ni se usa un parser YAML paralelo.

Prueba propuesta: matriz de integración contra `WorkspaceConfig::load` que observe el valor efectivo
de cada caso válido y rechace cada scalar inválido. Guarda anti-vacuidad: verificar que los scalars
con espacios en su contenido son rechazados, que el whitespace sintáctico no se rechaza ni altera el
valor, y que ningún rechazo cae al default ni crea un `MemoryBudget` exitoso.

### C3 — Mínimo y overflow

**Dado** `63MiB`, `64MiB` y una cadena sintácticamente válida cuya multiplicación binaria excede
`u64`;
**Cuando** se convierte el valor;
**Entonces** `63MiB` y el overflow fallan de forma accionable, `64MiB` tiene éxito y ninguna ruta
envuelve, trunca ni entra en pánico.

Prueba propuesta: tabla de límites con una magnitud decimal suficientemente grande para desbordar
cada factor aplicable. Guarda anti-vacuidad: afirmar el valor exacto de `64MiB` y el rechazo del
overflow, no solo que existe algún error.

### C4 — Camino de error existente y proyección MCP

**Dado** un fallo de C2 o C3 durante `Workspace::open`;
**Cuando** la fachada MCP proyecta ese fallo;
**Entonces** conserva el camino de error existente y lo expone como `INTERNAL_IO_ERROR`, con el
mensaje accionable, sin añadir código, campo ni variante al contrato.

Prueba propuesta: integración de `lodestar-app`/MCP que abre un root con config inválida y observa
el código y mensaje wire. Guarda anti-vacuidad: assert de `INTERNAL_IO_ERROR` y assert negativo de
`INVALID_SCHEMA`/éxito silencioso; además verificar que `contracts/mcp.yml` no cambia.

### C5 — Independencia del host y compatibilidad por adición

**Dado** el mismo texto válido en dos workspaces aislados y una configuración anterior que omite
la sección `performance`;
**Cuando** se abre cada workspace sin consultar memoria disponible del host;
**Entonces** el resultado de bytes es idéntico y la configuración anterior usa `256MiB`, con
independencia de RSS, cgroup o límites del proceso.

Prueba propuesta: prueba determinista de apertura con un proveedor de memoria del host ausente,
fallido y con valores distintos (si el arnés lo expone), más una fixture sin sección nueva. Guarda
anti-vacuidad: el caso ausente debe observar exactamente el default y el test falla si se intenta
leer una sonda de host; un binario antiguo rechazando el campo nuevo se documenta como asimetría
esperable, no como criterio de compatibilidad hacia atrás.

### C6 — Owner y construcción única

**Dado** un `Workspace::open` exitoso;
**Cuando** se inspecciona la creación y entrega del presupuesto durante esa apertura;
**Entonces** existe exactamente una construcción de `MemoryBudget`, cuyo owner es
`lodestar-workspace::Workspace::open`, y ningún core, store o fachada crea otro.

Prueba propuesta: integración con contador/identidad de construcción y verificación de ownership
entre crates, ejecutada con y sin `performance.maxMemory` explícito. Guarda anti-vacuidad: abrir dos
veces exige una construcción por apertura, mientras una apertura no puede producir dos presupuestos
ni un presupuesto global compartido entre workspaces.

### C7 — Partición exacta, redondeo y residuo protegido

**Dado** `N` en bytes, incluyendo valores cuyo residuo módulo 100 no es cero (por ejemplo `101`,
`199` y el `N` de `64MiB`);
**Cuando** se particiona `MemoryBudget`;
**Entonces** `SQLite = floor(30*N/100)`, `W-TinyLFU = floor(20*N/100)`, `Work = N-SQLite-WTinyLFU`,
la suma es exactamente `N` y `Work` es la reserva protegida que recibe todo residuo.

Prueba propuesta: tabla de propiedad sobre el particionador puro y una integración con `64MiB`.
Guarda anti-vacuidad: incluir `101`/`199` debe distinguir esta regla de `Work = floor(50*N/100)`;
assertar que una cache no puede consumir bytes de `Work` y que no aparece una cuarta reserva.

### C8 — Sin knobs ni consumidores runtime

**Dado** una config que intenta declarar presets o knobs internos (por ejemplo cuotas SQLite o
W-TinyLFU) y un workspace limpio;
**Cuando** se carga y se abre el workspace;
**Entonces** solo `performance.maxMemory` es aceptado, los knobs desconocidos se rechazan por el
camino estricto existente y la apertura no conecta SQLite ni activa W-TinyLFU/runtime posterior.

Prueba propuesta: fixture negativa de claves desconocidas y prueba de apertura que compruebe que no
se habilita store/cache ni se crean artefactos runtime. Guarda anti-vacuidad: assert negativo de
aceptación de cada knob y de creación de un consumidor; no basta con que el YAML sea parseable.

### C9 — Documentación, trazabilidad y deferencias

**Dado** este requisito y los seis documentos de autoridad tocados por la ratificación;
**Cuando** se ejecuta la revisión documental y de estado;
**Entonces** todos expresan la fórmula con `floor` y residuo a `Work`, enlazan la trazabilidad
`#53: E34-H01 → E35-H01`, mantienen §14 abierta, deferen fuera-cache/error/no-thrashing a
`#55/#57/#59/#62`, declaran ausencia de delta MCP y marcan E35-H01 como `ratificada` en el
frontmatter y `implementada` en el estado de entrega,
no `pendiente`.

Prueba propuesta: comprobación de consistencia (`git diff --check`, búsquedas de fórmula y
trazabilidad, y revisión del estado) sobre `ARCHITECTURE.md`, `docs/REFACTOR_PHASE_2.md`,
`docs/SCALABILITY_ANALYSIS.md`, `decisiones/README.md`, `decisiones/14-store-sin-consumidor.md`,
`IMPLEMENTATION_STATUS.md` y este requisito. Guarda anti-vacuidad: la comprobación falla si algún
documento normativo calcula `Work` directamente como la mitad de `N`, si §14 se marca cerrada, si se afirma implementación runtime
o si `contracts/mcp.yml` cambia.

## Dependencias

- Configuración YAML vigente y su política de claves desconocidas (`E15-H08`, `E29-H01`).
- `Workspace::open` como chokepoint de carga y owner único.
- `WorkspaceError::Io` y la proyección existente a `INTERNAL_IO_ERROR` en las fachadas.
- Autoridad de `ARCHITECTURE.md §23` y documentación de migración actualizada por esta ratificación.
- No depende de resolver `decisiones §14`; de hecho, debe dejarla abierta.

## Delta de contrato y documentación

- `contracts/mcp.yml`: **sin cambios**; no hay códigos, campos ni tools nuevos.
- Configuración: se documenta/admite `performance.maxMemory` con default, mínimo y gramática
  anteriores; el fallo conserva el camino existente.
- Documentación: se actualizan las seis autoridades enumeradas en C9 y este requisito.
- Estado: el frontmatter conserva `E35-H01` como `ratificada`; el estado real de entrega es
  `implementada`.

## Gates de la entrega

La entrega mantiene bloqueados los tests de la fase roja y conserva la secuencia de gates del
repositorio. Para esta materialización se exige: `git diff --check`,
`scripts/agent-gates.sh policy` y `scripts/agent-gates.sh contract`. No se crean ramas, commits,
pushes ni PRs.

## Texto de ratificación

> Ratifico **E35-H01 — Contrato de memoria retenida y configuración `performance.maxMemory`** el
> **2026-08-24**, trazada desde la issue #53 titulada `[E34-H01]` por la colisión histórica de ID.
> Ratifico default `256MiB`, mínimo `64MiB`, la gramática estricta aplicada al scalar YAML semántico
> deserializado (el whitespace sintáctico antes de newline, comentario, coma o `}` no forma parte
> del valor; un scalar con espacios sí es inválido) y conversión `u64` *checked*;
> `MemoryBudget` único construido por `lodestar-workspace::Workspace::open`; y la partición
> `SQLite=floor(30*N/100)`, `W-TinyLFU=floor(20*N/100)`, `Work=N-SQLite-WTinyLFU`, con todo residuo
> en `Work`, suma exacta `N` y reserva protegida. El alcance es únicamente configuración,
> contabilidad, mensajes, pruebas y documentación. No ratifico conexión del store, implementación
> de W-TinyLFU, runtime fuera-cache/error/no-thrashing, knobs, sondas RSS/cgroup, delta MCP ni una
> salida para `decisiones §14`. Estado normativo: **ratificada**; estado de entrega: **implementada**.
