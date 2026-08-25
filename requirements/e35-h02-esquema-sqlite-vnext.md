---
id: E35-H02
titulo: "Esquema SQLite vNext compacto por IDs"
estado: "ratificada"
ratificada_en: "2026-08-25"
origen: "GitHub issue #54, titulada originalmente [E34-H02]; parent #52"
trazabilidad: "La colisión histórica E34-H02 se resuelve materializando esta historia como E35-H02"
---

# E35-H02 — Esquema SQLite vNext compacto por IDs

## Objetivo observable

Tras reconstruir la cache derivada de un workspace Markdown no vacío, `.lodestar/index.db` usa un
esquema vNext relacional y compacto: cada documento tiene un identificador entero; metadata,
diagnósticos y enlaces referencian IDs donde el referente es conocido; las rutas de metadata
repetidas se normalizan en un diccionario; y FTS no conserva una tercera copia completa del corpus.

La reconstrucción conserva la paridad observable core↔store y un informe `dbstat` desglosa el coste
persistente de cada tabla, índice y objeto FTS. Markdown sigue siendo la única fuente de verdad: una
cache v5 o incompatible se descarta y se recrea, nunca se migra in-place ni modifica Markdown.

En vNext, `documents.body` conserva el snapshot Markdown completo y exacto (frontmatter y cuerpo
incluidos) como única copia completa de contenido en SQLite. FTS5 usa la variante elegida conforme a
C4/C5 y liga sus filas a `doc_id`; la evidencia versionada de 10.000 documentos selecciona
contentless (`content=''`, `columnsize=0`) frente a external-content (`524288 bytes` frente a
`651264 bytes` de objetos FTS medidos con `dbstat`; reducción `126976` por `docsize`). Cuando se usa el camino cacheado, `DocumentStore`
y el core leen el snapshot desde SQLite. Esto no traslada la autoridad: el
Markdown en disco sigue siendo canónico, SQLite es derivado/reconstruible y no es lectura por defecto
mientras §14 siga abierta.

La issue #54 conserva en GitHub el título original `[E34-H02]`. Ese ID colisiona con la historia ya
cerrada de interoperabilidad MCP; la trazabilidad local normativa es **E34-H02 → E35-H02**.

## Autoridades y referencias vigentes

- `ARCHITECTURE.md`, en especial los invariantes activos, §20/§20.12, §21.5, §22 y §23.
- `docs/REFACTOR_PHASE_2.md` y `ARCHITECTURE.md §20`, comportamiento de la migración.
- Diseño y descomposición ratificados de GitHub #52 y la historia GitHub #54.
- `requirements/epica-35-presupuesto-memoria.md`: E35-H01 está implementada; H02 no consume aún su
  `MemoryBudget`.
- `decisiones/14-store-sin-consumidor.md`: permanece abierta; H02 no conecta el store ni la cierra.
- `docs/qa/evidencia-14-store-2026-08.md` y
  `docs/qa/e33-h09-realista-100k-2026-08-23.md`: baseline Realista/100k y coste global actual.
- `crates/lodestar-store/src/schema.rs`: v5 conserva `body` y `raw`, repite paths textuales y usa
  FTS con contenido propio.

El prototipo histórico no es referencia normativa ni oráculo de paridad.

## Alcance

- Incrementar `user_version` e introducir el DDL vNext con recreación limpia de cache incompatible.
- Definir `documents(doc_id INTEGER PRIMARY KEY, path TEXT UNIQUE, …)`.
- Referenciar documentos mediante IDs enteros en metadata, diagnostics y enlaces cuando sea posible.
- Normalizar rutas de metadata en `fields(field_id, field_path UNIQUE)`, usando la representación
  anclada que publica el core.
- Conservar `target_path` para destinos no materializables y para invalidación dirigida futura;
  usar `target_doc_id` cuando el destino es un documento conocido.
- Eliminar la coexistencia de `raw` con un segundo `body` y ligar FTS a la columna `body` completa por
  `rowid = doc_id`, conforme a la variante seleccionada por el spike reproducible (contentless en la
  evidencia ratificada).
- Elegir FTS5 contentless o external-content mediante un spike reproducible y medición `dbstat`.
- Mantener la API interna vigente y la paridad core↔store sin convertir SQLite en fuente canónica.
- Añadir al banco un desglose `dbstat` reconciliable por objeto SQLite.
- Adaptar tests, fixtures, documentación interna, trazabilidad y estado afectados.

## Fuera de alcance

- Conectar SQLite a `App`/MCP, crear `KnowledgeIndex` o migrar las siete lecturas (#59/E35-H07).
- Rebuild streaming, `.next`, swap atómico, prepared statements o insert-only (#55/E35-H03).
- Watcher incremental, reconciliación dirigida o generaciones (#56/E35-H04 y #60/E35-H08).
- W-TinyLFU, page-cache, autotuning, reparto runtime de memoria u observabilidad de caches.
- Cambiar `DiscoveryPolicy` o el walker: se alinearán antes de conectar el store, dentro de #55.
- Cambiar contrato MCP/CLI, códigos de error, configuración pública o documentación externa.
- Convertir el objetivo Realista/100k de footprint ≤2,5× en gate; corresponde a #62/E35-H10.
- Cerrar o reabrir `decisiones §14`.

## Criterios BDD binarios y pruebas propuestas

### C1 — Cache incompatible se reconstruye, Markdown no se migra

**Dado** un índice v5 válido o un SQLite cuyo `user_version` y DDL son incompatibles;
**Cuando** se abre el store vNext;
**Entonces** la cache se descarta y recrea con una versión nueva y DDL completo, sin migrar filas
in-place ni modificar ningún Markdown.

Prueba propuesta: integración que crea v5 con una fila sentinela y un Markdown, abre vNext y verifica
que la sentinela no sobrevive, el Markdown es byte a byte idéntico, y `user_version`/`sqlite_master`
son vNext. Guarda: una versión con nombres parcialmente coincidentes pero DDL inválido también debe
reconstruirse.

### C2 — IDs enteros, integridad referencial y destino dirigido

**Dado** documentos con metadata, enlaces entre documentos, un enlace ausente y diagnostics locales;
**Cuando** se reconstruye el índice;
**Entonces** `documents` tiene `doc_id INTEGER PRIMARY KEY` y path único; metadata, diagnostics y
origen/destino conocido de links usan IDs; el destino ausente conserva `target_path` con
`target_doc_id IS NULL`.

Prueba propuesta: inspeccionar `table_info`, `foreign_key_list`, índices y filas. Guardas: IDs
distintos para documentos distintos, ausencia de `document_path`/`source_path` redundantes y ningún
huérfano relacional al borrar con `foreign_keys=ON`.

### C3 — Diccionario único de field paths anclados

**Dado** varios documentos que repiten campos reservados (`graph`, `document`) y ordinarios;
**Cuando** se indexan;
**Entonces** cada ruta aparece una sola vez en `fields`, metadata apunta por `field_id` y la forma
almacenada coincide exactamente con la forma anclada del core.

Prueba propuesta: comparar diccionario y joins con `FieldPath::anclado()`/catálogo core. Guardas:
ejercitar más de un documento y rechazar tanto la duplicación como conservar simultáneamente formas
crudas y ancladas.

### C4 — Un solo contenido y FTS ligado por doc_id

**Dado** un corpus no vacío con cuerpo, título y frontmatter textual;
**Cuando** se ejecuta el spike y se reconstruye vNext;
**Entonces** se implementa exactamente una variante contentless o external-content ligada por
`rowid = doc_id`, que devuelve candidatos reales; `documents.body` es el snapshot completo y el DDL
no conserva `raw` junto con un segundo `body` ni una copia propia completa de FTS.

Prueba propuesta: mismo corpus para ambas variantes del spike, consulta con una aguja exclusiva y
confirmación mediante el matcher del core; inspeccionar el DDL final y las shadow tables. Guardas:
FTS vacío, devolver todo o duplicar el cuerpo completo no satisface el criterio.

### C5 — Elección FTS respaldada por dbstat

**Dado** las dos variantes funcionales de C4 sobre el mismo corpus;
**Cuando** se recogen sus costes con `dbstat`;
**Entonces** se elige contentless si cumple C4 y su coste atribuible es menor o igual; external-content
solo si contentless no cumple C4 o cuesta más. Variante, números y justificación quedan documentados.

Prueba propuesta: el recolector emite objetos y total para ambos artefactos. Guardas: no seleccionar
sin informe, omitiendo shadow tables o usando corpus diferentes. Esta elección técnica ya fue
delegada por #52/#54 y no necesita otra ratificación.

### C6 — Paridad funcional core↔store

**Dado** las fixtures canónicas y casos con enlace roto, `workspaceFile`, metadata anidada y Unicode;
**Cuando** vNext se reconstruye o actualiza;
**Entonces** documentos, metadata, enlaces, backlinks, aislados, dangling, blast radius,
diagnostics/agregados y candidatos FTS confirmados son iguales a los del core.

Prueba propuesta: adaptar la paridad existente de `lodestar-store` y añadir una fixture que ejerza
C2–C4 conjuntamente. Guarda: no basta contar filas ni usar un corpus sin problemas semánticos.

### C7 — Desglose dbstat completo y reconciliado

**Dado** una base vNext no vacía con todas las familias de filas;
**Cuando** el banco produce su informe SQLite;
**Entonces** incluye bytes por tabla, índice y objetos FTS/shadow, más bytes no atribuibles, y la suma
se reconcilia exactamente con los bytes de la base principal en el punto de medición.

Prueba propuesta: test de formato sobre una DB temporal poblada, con enteros no negativos, objetos
no vacíos y suma exacta. Guarda: `main_bytes`/WAL/SHM sin desglose no satisface el criterio.

### C8 — Footprint es objetivo, no gate ni promesa externa

**Dado** una medición de footprint vNext;
**Cuando** se compara con los bytes Markdown y el target Realista/100k ≤2,5×;
**Entonces** el informe conserva ese target como objetivo de ingeniería no bloqueante y no presenta
SQLite como camino de lectura por defecto mientras §14 siga abierta.

Prueba propuesta: comprobar estructuralmente la distinción `objective`/`gate` y revisar §21.5,
§14 y la documentación pública. Guarda: la mera presencia textual de `2.5` no basta.

## Dependencias y orden

- Diseño/descomposición de #52 ratificados; E18 completada; E35-H01 implementada.
- E35-H02 es fundación de #55/E35-H03 y #56/E35-H04; #59/E35-H07 consume sus IDs e índices.
- El cierre de §14, activación por defecto y gates 100k/1M permanecen posteriores.

## Delta de contrato y documentación

- `contracts/mcp.yml`, wire MCP/CLI, códigos y configuración: **sin delta**.
- Documentación interna: DDL vNext en `ARCHITECTURE.md`, formato `dbstat`, evidencia del spike,
  trazabilidad de #54 y `IMPLEMENTATION_STATUS.md` cuando la entrega esté verificada.
- Documentación externa: sin delta; se conserva la honestidad de `ARCHITECTURE.md §21.5`.

## Texto de ratificación

> Ratifico **E35-H02 — Esquema SQLite vNext compacto por IDs** el **2026-08-25**, trazada desde la
> issue #54 titulada `[E34-H02]`. Ratifico `doc_id INTEGER PRIMARY KEY`, relaciones por IDs,
> diccionario `fields`, `target_path` para invalidación dirigida, ausencia de duplicación completa
> `raw + segundo body + FTS-content`, snapshot Markdown completo en `documents.body`, FTS contentless o
> external-content elegido mediante spike y `dbstat`,
> rebuild de cache desechable, paridad core↔store y desglose de footprint. El objetivo Realista/100k
> ≤2,5× no es gate hasta medir. No ratifico conexión del store, rebuild streaming, watcher dirigido,
> cache RAM, cambios MCP/CLI/configuración ni el cierre de §14. Estado normativo: **ratificada**.
