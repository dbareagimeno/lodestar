---
id: E35-H03
titulo: "Rebuild SQLite streaming, insert-only y atómico"
estado: "implementada"
ratificada_en: "2026-08-25"
origen: "GitHub issue #55, titulada originalmente [E34-H03]; parent #52"
trazabilidad: "La colisión histórica E34-H03 se resuelve materializando esta historia como E35-H03"
---

# E35-H03 — Rebuild SQLite streaming, insert-only y atómico

## Objetivo observable

Al reconstruir la cache SQLite de un workspace Markdown, Lodestar construye
`.lodestar/index.db.next` en streaming, sin retener los cuerpos del corpus completo ni borrar
documentos durante la carga. Solo publica el índice nuevo tras verificar su integridad mediante un
swap atómico.

Si el rebuild falla o se interrumpe antes del swap, el índice activo anterior permanece íntegro y
consultable. La reconstrucción aplica la `DiscoveryPolicy` efectiva de la sesión y conserva paridad
exacta core↔store. Markdown continúa siendo la fuente canónica y `decisiones §14` permanece abierta:
esta historia mejora una cache derivada, pero no la conecta al camino normal de lectura.

La issue #55 conserva en GitHub el título original `[E34-H03]`. Ese ID colisiona con la historia ya
cerrada de interoperabilidad MCP; la trazabilidad local normativa es **E34-H03 → E35-H03**.

## Autoridades y referencias vigentes

- `ARCHITECTURE.md`, en especial los invariantes activos, §20, §20.12.1, §21.5 y §23.
- `docs/REFACTOR_PHASE_2.md`, comportamiento de descubrimiento, cache derivada y migración.
- Diseño y descomposición ratificados de GitHub #52, y GitHub #55 como historia de origen.
- `requirements/e35-h02-esquema-sqlite-vnext.md`: E35-H02 es la dependencia ya implementada.
- `decisiones/14-store-sin-consumidor.md`: permanece abierta; H03 no conecta el store ni la cierra.
- `docs/SCALABILITY_ANALYSIS.md`: estrategia de inventario e indexación streaming.
- `IMPLEMENTATION_STATUS.md`: E35-H01 y E35-H02 están implementadas.

El prototipo histórico no es referencia normativa ni oráculo de paridad.

## Alcance

- Reconstruir una generación nueva en `.lodestar/index.db.next`, en el mismo directorio de la cache
  activa.
- Ejecutar dos pasadas streaming: una para inventario de paths y entradas necesarias para resolver
  enlaces, y otra para leer, parsear y proyectar un documento cada vez, liberando su payload al
  terminar.
- Aplicar la misma `DiscoveryPolicy` efectiva que usa la sesión de workspace, incluidos ignores,
  `include`/`exclude`, límites y exclusión del plano de control.
- Reutilizar statements preparados durante la construcción.
- Construir la base nueva mediante una ruta insert-only, sin `DELETE` por documento, path o FTS.
- Usar pragmas seguros para una base desechable todavía no publicada, sin degradar la integridad de
  la generación activa.
- Ejecutar `integrity_check` o una comprobación SQLite equivalente antes de publicar.
- Publicar únicamente una base completa y válida mediante swap atómico.
- Medir el rebuild en 1k, 10k y 100k, con duración, pico RSS y contadores por fase que permitan
  detectar trabajo repetido o crecimiento superlineal.
- Mantener paridad core↔store y completar pruebas, documentación interna, trazabilidad y estado.

## Fuera de alcance

- Conectar SQLite a `App`, CLI o MCP; crear `KnowledgeIndex`; cambiar el camino normal de lectura;
  cerrar o escoger una opción de `decisiones §14`.
- Watcher incremental, reconciliación dirigida o generaciones persistentes posteriores
  (#56/E35-H04 y #60/E35-H08).
- W-TinyLFU, paginación, rutas fuera de cache y política completa anti-*thrashing* de #57/#59/#62.
- Cambiar el DDL vNext, FTS contentless, `dbstat`, configuración pública o la partición de
  `MemoryBudget` de E35-H01.
- Cambiar wire MCP/CLI, tools, códigos de error o `contracts/mcp.yml`.
- Convertir en gates los objetivos Realista/100k de 60 s y 512 MiB RSS sin una ratificación
  posterior que fije también máquina, configuración y método de medida.

## Criterios BDD binarios y pruebas propuestas

### C1 — Dos pasadas sujetas a la política canónica

**Dado** un workspace con `.gitignore`, `.lodestarignore`, `include`/`exclude`, un Markdown que
supera `maxDocumentBytes`, documentos admitidos y assets enlazables; **cuando** se reconstruye la
cache; **entonces** el inventario y las filas indexadas coinciden exactamente con el descubrimiento
canónico: no entra ni se lee contenido excluido o no admitido, y un asset admitido conserva su
clasificación `workspaceFile`.

Prueba propuesta: integración con una aguja FTS en un Markdown excluido y un asset cuya presencia
distingue `workspaceFile` de `missing`. Guarda: contar documentos sin comprobar FTS, clasificación y
límites no satisface el criterio.

### C2 — Streaming con memoria acotada

**Dado** un corpus de cuerpos grandes cuyo tamaño total supera ampliamente el trabajo simultáneo;
**cuando** se reconstruye; **entonces** ningún `Vec` o mapa retiene los cuerpos del corpus, cada
payload se libera después de proyectarse y la memoria de trabajo no crece con los bytes Markdown
totales.

Prueba propuesta: integración en proceso aislado con corpus escalado y observación del pico durante
el rebuild, más una guarda estructural sobre el tipo de inventario. Guardas: centinelas al principio
y final, y medición durante la fase; medir solo RSS final o usar documentos vacíos sería vacuo.

### C3 — Carga insert-only y statements reutilizados

**Dado** una `.next` recién creada y un corpus con metadata, links, diagnostics y FTS; **cuando** se
carga; **entonces** la fase de construcción usa inserciones/proyecciones sobre el snapshot nuevo,
no ejecuta `DELETE` por documento, path o FTS, y el número de preparaciones SQL no crece con el
número de documentos.

Prueba propuesta: traza o seam equivalente sobre dos escalas, exigiendo filas reales de todas las
familias y FTS consultable. Guardas: cualquier delete prohibido o preparaciones proporcionales a N
hace fallar la prueba.

### C4 — Coste aproximadamente lineal y evidencia por fase

**Dado** corpus Realista generado a 1k, 10k y 100k; **cuando** corre el rebuild instrumentado;
**entonces** documentos leídos, proyecciones e inserciones FTS/relacionales crecen linealmente con N,
sin reprocesar documentos ya completados, y el informe conserva duración y pico RSS por fase.

Prueba propuesta: banco reproducible con contadores de trabajo y metadatos de máquina. Guardas: la
duración total sola no demuestra ausencia de trabajo cuadrático; el informe debe separar RSS
diagnóstico de la memoria retenida/controlable de §23.

### C5 — Integridad antes de swap

**Dado** un índice activo válido y una `.next` completa; **cuando** la nueva base supera la
comprobación de integridad; **entonces** se publica mediante un único swap atómico y el `index.db`
publicado contiene exactamente el snapshot nuevo.

Prueba propuesta: sentinelas distintos en el índice activo y el corpus fuente; abrir el
`index.db` final y consultar el sentinela nuevo. Guarda: la mera existencia de `.next` no satisface
el criterio.

### C6 — Interrupción segura

**Dado** un índice activo válido; **cuando** el rebuild falla antes del swap o la comprobación de
integridad de `.next` no es satisfactoria; **entonces** el activo conserva el snapshot anterior,
sigue abriéndose y consultándose, `.next` parcial no se adopta, y Markdown queda byte a byte igual.

Prueba propuesta: seam/failpoint inmediatamente anterior a publicar y otro que fuerce fallo de
integridad. Guardas: reabrir y consultar el sentinela anterior, no limitarse a comprobar que el
fichero existe.

### C7 — Paridad semántica exacta

**Dado** fixtures con Unicode, metadata anidada, enlaces internos y rotos, `workspaceFile`,
diagnósticos y paths filtrados; **cuando** se reconstruye; **entonces** documentos, metadata,
clasificación de links, agregados, diagnósticos y candidatos FTS confirmados coinciden exactamente
con el core.

Prueba propuesta: extender la paridad de `lodestar-store` con una fixture conjunta E35-H02/H03.
Guarda: comparar valores y conjuntos, no solo conteos ni un corpus trivial.

### C8 — Objetivos 100k medidos, no gate prematuro

**Dado** una corrida Realista a 1k, 10k y 100k; **cuando** el banco informa el rebuild; **entonces**
incluye los objetivos `rebuild <= 60 s` y `peak RSS <= 512 MiB` como objetivos de ingeniería
no bloqueantes, junto con los valores observados y la procedencia; no los etiqueta como gates.

Prueba propuesta: test de formato del informe y evidencia reproducible. Guarda: la mera presencia
textual de las cifras, sin `gate: false`, medidas ni procedencia, no satisface el criterio.

## Dependencias y orden

- E35-H01 — contrato de `performance.maxMemory` y `MemoryBudget`: implementada.
- E35-H02 — esquema SQLite vNext compacto por IDs: implementada.
- Descubrimiento universal y `DiscoveryPolicy` vigentes.
- Esta historia habilita, pero no implementa, #56/E35-H04, #57, #59/E35-H07 y #62.

## Delta de contrato y documentación

- `contracts/mcp.yml`, wire MCP/CLI, tools, códigos de error y configuración pública: **sin delta**.
- Documentación interna: arquitectura del rebuild, guía/evidencia del benchmark, trazabilidad de #55,
  `decisiones §14` e `IMPLEMENTATION_STATUS.md` cuando la entrega esté verificada.
- Documentación externa: sin delta; SQLite sigue fuera del camino de lectura por defecto mientras
  `decisiones §14` permanezca abierta.

## Ratificación

Ratificada el 2026-08-25 por la petición explícita de implementar GitHub #55. Se ratifican las
decisiones cerradas de la issue y los criterios C1–C7. C8 conserva el estado que la propia issue
declara: 60 s y 512 MiB son objetivos hasta una ratificación posterior del gate y de sus condiciones
de medida.

## Evidencia de implementación

Implementada y verificada el 2026-08-26. La entrega usa inventario canónico compacto que no abre
cuerpos y una segunda pasada que lee cada candidato una vez. Los UTF-8 válidos se parsean una sola
vez y reutilizan ese `Parsed`; los inválidos pasan a `other_files`. La promoción `O(log N)` reata
enlaces adelantados usando la semántica canónica de `LinkTarget`. Construye `index.db.next` insert-only con statements
reutilizados, valida integridad y publica por rename atómico con sincronización del fichero y el
directorio. El writer gate serializa rebuild e incrementales mientras las lecturas conservan la
generación activa. Los tests H03 de store, workspace y benchmark cubren política, memoria, SQL,
interrupción, concurrencia, paridad y RSS específico del rebuild. `contracts/mcp.yml` no cambia y
`decisiones §14` continúa abierta.

La matriz de verificación de implementación Realista 1k/10k/100k y su método reproducible quedan resumidos en
[`docs/qa/e35-h03-rebuild-streaming-2026-08-26.md`](../docs/qa/e35-h03-rebuild-streaming-2026-08-26.md).
