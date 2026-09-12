# Análisis de escalabilidad de Lodestar

## Contexto

Los benchmarks actuales son asumibles hasta ~10.000 documentos. Extrapolando los resultados observados, un corpus de 1.000.000 de documentos podría alcanzar aproximadamente 10 GB de RAM y unos 50 segundos de rebuild en un M4 base.

Estas cifras son una extrapolación histórica de mediciones de proceso y no un contrato de memoria:
RSS no es la métrica normativa del producto. La adenda ratificada **E35-H01** (issue #53, titulada
originalmente **E34-H01** y trazada localmente como **E34-H01 → E35-H01**) fija el presupuesto de memoria retenida y controlable en
`ARCHITECTURE.md §23`; la implementación de ese alcance está entregada y verificada en los tests
dirigidos de E35-H01.

Un millón de documentos no tiene por qué ser el caso habitual, pero sí es una escala empresarial plausible. El problema principal no es si SQLite puede almacenar esa cantidad de filas, sino cuántas representaciones del corpus mantiene Lodestar simultáneamente y cuántas operaciones recorren el corpus completo.

La propiedad objetivo debería ser:

> El rebuild completo sigue siendo proporcional al tamaño del corpus, pero la memoria queda limitada por una cuota fija; el arranque normal reutiliza el índice existente y las operaciones cuestan en función de los documentos afectados, no del corpus completo.

## Presupuesto ratificado de memoria (E35-H01)

La configuración pública es únicamente `performance.maxMemory` en `.lodestar/config.yaml`:

```yaml
performance:
  maxMemory: 256MiB
```

Su ausencia usa **256 MiB** y el mínimo es **64 MiB**. El scalar YAML semántico, una vez
deserializado, debe satisfacer exactamente `[1-9][0-9]*(MiB|GiB)`, case-sensitive, sin espacios en
su contenido, fracciones ni ceros iniciales. El whitespace sintáctico separado antes de newline,
comentario, coma o `}` no integra el valor; por ello `performance: {maxMemory: 256MiB }` equivale
a `256MiB`. Sí son inválidos `"256MiB "`, `" 256MiB"` y `256 MiB`, además de `0MiB`, `064MiB`,
`1.5GiB`, `256mib` y `256MB`. La conversión a bytes usa `u64` y aritmética *checked*; un valor
inválido, menor que el mínimo o que desborde se rechaza con un mensaje accionable por el camino de
error existente, sin delta en MCP. No se introduce un scanner/lexer source-aware ni un parser YAML
paralelo. No se consulta cgroup ni RSS al abrir.

Sea `N` el total de `performance.maxMemory`, contado como memoria retenida y controlable, no RSS.
SQLite dispone de una cuota interna blanda `floor(30 * N / 100)` y W-TinyLFU de otra
`floor(20 * N / 100)`. `Work = N - SQLite - W-TinyLFU` es la reserva protegida dentro de `N`,
recibe todo residuo y las caches nunca la invaden. Las tres partes agotan `N` exactamente: no existe
una cuarta reserva, un pool sin tope ni un tamaño abierto. El
único owner de `MemoryBudget` es `lodestar-workspace::Workspace::open`; core, store y fachadas no lo
crean ni lo poseen. Las cuotas no son límites públicos ni crean presets o knobs adicionales.
W-TinyLFU es un rol de cache futuro: esta adenda no implementa la cache ni conecta SQLite al camino
de lectura.

`discovery.maxDocumentBytes` sigue siendo admisión documental. La semántica ratificada permite que
un documento admitido se procese fuera de cache si es seguro; si no lo es, debe fallar explícitamente
por el camino existente, sin reintentos que produzcan *thrashing*. La implementación efectiva de
esas rutas corresponde a las issues posteriores **#55, #57, #59 y #62**, según la historia concreta;
E35-H01 no promete ejecutarlas. Configs antiguas omiten el campo nuevo y usan el default; binarios
antiguos pueden rechazarlo. El detalle normativo está en
[`ARCHITECTURE.md §23`](../ARCHITECTURE.md#23-presupuesto-de-memoria-retenida-e35-h01).

## Diagnóstico del diseño actual

Lodestar ya dispone de SQLite/FTS5 en `lodestar-store`, pero todavía no actúa como columna vertebral de todas las operaciones normales.

Puntos relevantes:

1. La apertura normal mediante `Workspace::open` deja la cache desactivada.
2. `Workspace::open_live`/`enable_cache` activan la cache, pero actualmente ejecutan un rebuild completo.
3. `walk_disk()` acumula todos los Markdown, incluido su contenido, antes de indexarlos.
4. `DocumentSet` mantiene simultáneamente el `FileMap`, documentos parseados, inventario y, bajo demanda, un `Analysis` completo.
5. La planificación de cambios crea un `working_files` a partir del `FileMap` completo.
6. El esquema histórico almacenaba simultáneamente `raw`, un `body` parseado y `frontmatter_json`;
   E35-H02 lo reemplaza localmente por `documents(doc_id, body, frontmatter_json, ...)`, sin `raw`.
   `documents.body` conserva el snapshot Markdown completo y exacto, FTS5 contentless (`content=''`,
   `columnsize=0`) recibe esa columna con `rowid = doc_id` manual, y el snapshot cacheado lo consumen
   `DocumentStore` y el core.
7. Algunas operaciones servidas desde SQLite vuelven a reconstruir estructuras globales del core para mantener la paridad semántica.
8. La búsqueda exacta de subcadena termina leyendo y parseando todas las filas candidatas del corpus.

Por tanto, una extrapolación de varios GB de RAM para 1M de documentos es compatible con una amplificación estructural del corpus, no necesariamente con una fuga de memoria.

## SQLite frente a Redis

### SQLite

SQLite debería ser el índice persistente y el motor principal de consultas de Lodestar.

Usos adecuados:

- índice derivado persistente;
- metadata;
- búsqueda textual mediante FTS5;
- enlaces y backlinks;
- diagnósticos materializados;
- revisiones y generaciones del índice;
- agregados y conteos;
- invalidación incremental.

El Markdown continúa siendo la fuente canónica. SQLite sigue siendo derivado y reconstruible.

### Redis

Redis no debería utilizarse para almacenar el corpus completo ni como índice principal. Al mantener el dataset en memoria, trasladar allí documentos, metadata y grafo probablemente aumentaría el footprint en lugar de reducirlo.

Solo tendría sentido como componente empresarial opcional cuando exista un despliegue distribuido:

- cache de resultados calientes con TTL;
- colas de trabajos de indexación;
- publicación de invalidaciones entre nodos;
- leases o coordinación entre workers.

El core y el funcionamiento local de Lodestar no deberían depender de Redis.

## Estrategia propuesta

### 1. Reutilizar el índice entre arranques

El cambio de mayor retorno es dejar de reconstruir automáticamente una cache válida.

SQLite debería mantener una cabecera de control equivalente a:

- `schema_version`;
- `index_format_version`;
- `parser_version`;
- hash de la política de descubrimiento;
- identidad del workspace;
- generación activa;
- indicador de build completo;
- instante/estado de la última reconciliación.

Si la generación existente es compatible, el arranque debería:

1. abrir inmediatamente el índice;
2. comenzar a servir consultas;
3. iniciar una reconciliación incremental;
4. reindexar únicamente documentos nuevos, modificados o eliminados.

### 2. Índices generacionales

Actualmente un cambio incompatible de esquema fuerza un rebuild limpio. A gran escala es preferible construir una nueva generación sin destruir la anterior:

```text
index-v5.db        # generación activa
index-v6.building  # nueva generación en construcción
```

Cuando la nueva generación está completa y reconciliada, el puntero activo cambia de forma atómica. Si el proceso falla, la generación anterior sigue disponible.

Esto convierte el rebuild completo en mantenimiento, no en downtime.

### 3. Rebuild en streaming y con memoria acotada

`walk_disk()` no debería devolver todos los contenidos en un `Vec`.

Propuesta en dos pasadas:

#### Primera pasada: inventario

Recorrer el filesystem sin abrir cuerpos y conservar únicamente información compacta. Un path
Markdown admitido por policy/tamaño todavía es un candidato de codificación desconocida:

- path;
- tipo de entrada;
- mtime;
- tamaño;
- hash conocido cuando exista.

#### Segunda pasada: indexación

La segunda pasada procesa cada candidato exactamente una vez:

1. toma un path;
2. lee su payload una sola vez y valida UTF-8 sobre esos mismos bytes;
3. si es inválido, lo reclasifica como `other_files` y no lo parsea ni proyecta;
4. si es válido, lo parsea una sola vez mediante el core;
5. reutiliza ese `Parsed` para metadata, links y diagnósticos locales;
6. produce la proyección indexable y libera el documento.

En E35-H03 el escritor es secuencial. Los candidatos posteriores se representan provisionalmente
como `WorkspaceFile`; al demostrar UTF-8 se promocionan en el inventario en `O(log N)` y un statement
preparado reata los enlaces previos con semántica derivada de `LinkTarget`.

La tubería debería tener varios workers de lectura/parseo, una cola limitada por bytes, un único escritor SQLite y backpressure.

La memoria pasa así de depender del número de documentos a depender de una cuota configurable:

```text
cache SQLite
+ cola de indexación
+ documentos en procesamiento
+ cache interna de documentos calientes (rol W-TinyLFU futuro)
```

E35-H01 ratifica el presupuesto explícito y su default; su alcance incluye la contabilidad y los
tests de memoria retenida/controlable por categoría. Las mediciones de RSS pueden acompañar la
evidencia, pero no sustituyen el contrato ni se validan contra cgroup al abrir.

### 4. Compactar el esquema SQLite

La compactación de IDs y la eliminación de la duplicación completa `raw`/`body` ya está materializada por
**E35-H02** (issue #54, **E34-H02 → E35-H02**). El DDL vigente, el spike FTS y el informe `dbstat`
están descritos en [`ARCHITECTURE.md §20.12.1`](../ARCHITECTURE.md#20121-adenda-ratificada-e35-h02--esquema-sqlite-vnext-por-ids-issue-54)
y [`docs/qa/e35-h02-fts-spike-2026-08-25.md`](qa/e35-h02-fts-spike-2026-08-25.md). Los párrafos y el
modelo conceptual siguientes conservan las alternativas de diseño que precedieron a esa ratificación;
no describen un trabajo pendiente de E35-H02.
El informe de footprint conserva `objective.max_ratio = 2.5`, `gate = false` y `read_default = false`;
el límite es un objetivo no bloqueante y SQLite sigue fuera de la lectura por defecto mientras §14
esté abierta.

El esquema histórico repetía paths y contenido. A gran escala conviene utilizar identificadores enteros internos.

Ejemplo conceptual:

```text
documents
  id INTEGER PRIMARY KEY
  path TEXT UNIQUE
  content_hash BLOB
  mtime_ns INTEGER
  size INTEGER
  title TEXT
  generation INTEGER

metadata
  document_id INTEGER
  field_id INTEGER
  value_type INTEGER
  value...

links
  source_id INTEGER
  target_id INTEGER NULL
  target_key_id INTEGER NULL
  kind INTEGER
  fragment TEXT

diagnostics
  document_id INTEGER
  code_id INTEGER
  severity INTEGER
  range...
```

Cambios especialmente interesantes:

- no repetir `document_path` textual en todas las tablas secundarias;
- internar nombres de campos repetidos;
- conservar un único snapshot Markdown completo en `documents.body`, y derivar de él
  `frontmatter_json`/`frontmatter_text` sin añadir otra copia completa;
- evitar almacenar simultáneamente `raw` y un segundo `body` parseado.

E35-H02 no elige la opción de omitir el contenido Markdown del índice: el snapshot completo en
`documents.body` permite que `DocumentStore` y el core lean la cache sin una segunda lectura de disco.
El Markdown en disco sigue siendo la fuente canónica; la versión/hash de la cache se valida y se
reconstruye cuando corresponde, y SQLite no se convierte en lectura por defecto mientras §14 esté
abierta.

Para fuentes futuras que no vivan como archivos locales podría existir un almacén de blobs separado, comprimido y direccionado por hash.

### 5. Hacer que FTS5 elimine realmente el escaneo completo

La búsqueda actual conserva la semántica exacta del core, pero acaba recorriendo el corpus.

Para mantener búsqueda por subcadena puede estudiarse el tokenizer `trigram` de FTS5:

1. FTS devuelve candidatos;
2. el core verifica la coincidencia exacta únicamente sobre esos candidatos;
3. se leen solo los resultados necesarios para completar la página y producir snippets.

E35-H02 ya selecciona FTS5 contentless (`content=''`, `columnsize=0`) y liga cada `rowid` a
`documents.doc_id` de forma manual; la consulta de candidatos hace `JOIN documents` antes de la
confirmación exacta del core. Quedan para historias posteriores el tokenizer trigram y opciones como
`detail`, siempre mediante benchmarks y sin romper el contrato de búsqueda. El protocolo de update y
delete conserva los valores antiguos exactos de `documents.body` (y las demás columnas indexadas)
antes de emitir el comando FTS5 correspondiente.

Las búsquedas globales deben ser paginadas y tener límites estrictos. Nunca deberían materializar cientos de miles de resultados en memoria.

### 6. Diagnósticos de enlaces incrementales

Los diagnósticos de enlaces no se materializan actualmente porque dependen del inventario global. En lugar de reconstruir el workspace completo, puede mantenerse un índice inverso de dependencias.

Para cada enlace se guarda una clave de destino normalizada por el core. Cuando aparece, desaparece o cambia una ruta:

1. SQLite localiza qué documentos enlazan potencialmente a esa clave;
2. solo esos documentos vuelven a pasar por el clasificador de enlaces del core;
3. los diagnósticos afectados se actualizan.

SQLite identifica candidatos; el core sigue siendo la autoridad semántica.

El coste pasa a depender del radio real de invalidación. Si 500.000 documentos apuntan a un destino modificado, actualizar esos 500.000 es correcto porque el cambio afecta realmente a todos.

### 7. Sustituir el `DocumentSet` global como unidad runtime

`DocumentSet` es adecuado como agregado puro, oráculo de paridad y modelo para corpus pequeños, pero no debería ser obligatorio materializarlo entero para un workspace de escala empresarial.

Mantenerlo para:

- tests;
- corpus pequeños;
- snapshots acotados;
- pruebas de paridad del store.

Añadir una vista perezosa respaldada por el store, conceptualmente:

```text
exists(path)
read(path)
metadata(path)
outgoing(path)
incoming(path)
diagnostics(path)
search(query, cursor, limit)
scan_documents(cursor, limit)
```

El core puede seguir siendo puro mediante interfaces de lectura. SQLite no reimplementa la semántica: proporciona datos y candidatos al core.

### 8. Planificación mediante overlay copy-on-write

`change_plan` no debería clonar el corpus completo para simular unas pocas operaciones.

Modelo conceptual:

```text
base_generation = 187
modified_documents = {
  "adr/42.md": nuevo_contenido
}
deleted_documents = {}
created_documents = {}
```

Las lecturas caen al índice base salvo para los documentos presentes en el overlay.

La memoria de una planificación depende así del cambio, no del tamaño total del corpus.

La validación del resultado puede combinar:

- diagnósticos de la generación base;
- documentos modificados;
- documentos afectados por cambios de enlaces;
- deltas sobre agregados globales.

### 9. Revisión global incremental

La revisión del workspace tampoco debería requerir releer todo el corpus para cada plan/apply.

Mantener un hash por documento y una raíz agregada. Puede implementarse inicialmente mediante buckets estables:

```text
bucket 0000 -> hash de documentos ordenados
bucket 0001 -> hash de documentos ordenados
...
workspace_root -> hash de los buckets
```

Modificar un documento recalcula su bucket y la raíz. Una evolución posterior podría utilizar un árbol Merkle completo si aporta ventajas adicionales.

El watcher mantiene el índice vivo, mientras que una reconciliación periódica o posterior a interrupciones protege frente a eventos perdidos.

### 10. Separar escritor y lectores SQLite

El store actual serializa el acceso alrededor de una única conexión protegida por `Mutex`.

Para una ejecución persistente:

- una conexión/hilo dedicado de escritura;
- un pequeño pool de conexiones de solo lectura;
- transacciones de lectura sobre una generación consistente;
- cache de páginas explícitamente presupuestada.

El índice debería vivir en almacenamiento local rápido. No debe compartirse un único SQLite mediante NFS entre varios hosts.

Puede ser conveniente permitir que la cache viva fuera del workspace:

```text
~/.cache/lodestar/<workspace-fingerprint>/index.db
```

Esto evita imponer la ubicación física del índice derivado al proyecto canónico.

### 11. No inventariar todos los archivos no Markdown si no es necesario

En un monorepo empresarial puede haber muchos más archivos de código que documentos Markdown.

Alternativa:

1. inventariar globalmente los documentos Markdown;
2. cuando un enlace apunta a un archivo no Markdown, realizar un `stat` seguro y acotado a la raíz;
3. cachear únicamente los archivos no Markdown realmente referenciados;
4. mantener información adicional de casing solo donde sea necesaria.

Esto evita que un repositorio enorme obligue a materializar todo `other_files` aunque Lodestar apenas haga referencia a esos archivos.

### 12. Particionar después de arreglar el modelo local

No introducir sharding o Redis para ocultar recorridos globales evitables.

Una vez que el modelo incremental esté resuelto, un workspace lógico empresarial podría particionarse por:

- repositorio;
- fuente;
- equipo;
- tenant;
- knowledge root.

Cada partición puede tener su SQLite y su generación. Una capa superior fusionaría los mejores resultados de las particiones relevantes.

## Papel concreto de Redis en una futura edición distribuida

### Cache de resultados

Clave conceptual:

```text
workspace_id + index_generation + query_hash
```

Con TTL y límites de memoria. Cambiar la generación invalida lógicamente las entradas antiguas sin una invalidación masiva.

### Cola de indexación

Redis puede distribuir trabajos entre workers cuando una instalación tenga varios nodos de procesamiento.

### Coordinación

Puede utilizarse para leases o elección del indexador activo. La protección real de una publicación debe seguir dependiendo de revisiones deterministas y del mecanismo transaccional durable de Lodestar.

### Lo que Redis no debe almacenar como única copia

- cuerpos del corpus;
- documentos parseados completos;
- grafo completo;
- índice durable;
- planes o recibos.

En instalaciones locales o de un solo nodo Redis no debería ser necesario.

## Roadmap recomendado

### Fase 1 — Eliminar coste repetido

1. Instrumentar tiempos separados de walk, lectura, parseo, proyección, SQLite y FTS.
2. Reutilizar una generación válida entre arranques.
3. Eliminar el rebuild automático de una cache válida.
4. Conectar las operaciones persistentes de MCP/servicios al store cuando corresponda.

### Fase 2 — Memoria acotada

5. ✅ E35-H03: rebuild en dos pasadas streaming con inventario compacto sin abrir cuerpos;
   clasificación UTF-8, parseo y proyección ocurren una sola vez por candidato en la segunda pasada.
   Los fingerprints de raíz y su destino real, entradas y frontera de directorios nacen en discovery
   y se revalidan hasta el swap para detectar cambios sin un tercer walker, manteniendo un body vivo.
6. Presupuesto `performance.maxMemory`, cola limitada por bytes y contabilidad de memoria retenida.
7. Escritor SQLite dedicado y lectores separados.
8. Paginación obligatoria para resultados potencialmente grandes.

### Fase 3 — Eliminar recorridos globales

9. FTS trigram como generador de candidatos para búsqueda por subcadena.
10. Diagnósticos de enlaces materializados con invalidación inversa.
11. Conteos y estado agregados servidos incrementalmente.
12. Planificación mediante overlay copy-on-write.
13. Revisión global incremental por hashes.

### Fase 4 — Compactación

14. ✅ E35-H02: IDs enteros para documentos, campos y destinos materializables.
15. ✅ E35-H02: eliminar la duplicación completa de `raw`/`body` y conservar un único snapshot
    Markdown completo y exacto en `documents.body`.
16. ✅ E35-H02: comparar FTS contentless/external-content mediante spike y seleccionar
    contentless (`content=''`, `columnsize=0`); el test versionado de 10.000 documentos con snapshot
    Markdown y frontmatter no vacío midió `524288 bytes` frente a `651264 bytes`, con body exclusivo
    `[4201]`, frontmatter exclusivo `[9877]`, `shared_count=10000` y ciclo de update/delete. La
    reducción es `126976 bytes` (`docsize` de external).
    Las optimizaciones posteriores de tokenizer/detail siguen abiertas.
17. ✅ E35-H03: construcción `index.db.next` con authorizer insert-only/prepares reconciliados,
    writer gate nativo interproceso, validación y swap atómico sin destruir antes el activo.

### Fase 5 — Escala empresarial distribuida

18. Particiones físicas por workspace/tenant/fuente cuando sean necesarias.
19. Cache SQLite en almacenamiento local, nunca compartida entre hosts mediante NFS.
20. Redis opcional para cache, trabajos y coordinación.

## Benchmarks que faltan

Mantener ~10k documentos en la suite habitual, añadir ~100k a ejecuciones más costosas/nocturnas y reservar ~1M para benchmarks manuales o de release.

Medir:

- tiempo y memoria retenida/controlable por categoría del rebuild frío;
- apertura con índice válido;
- reconciliación sin cambios;
- reconciliación con 0,1 % y 1 % de documentos modificados;
- modificación de un único documento;
- búsqueda selectiva;
- búsqueda con término muy común;
- creación de un documento que resuelve muchos enlaces rotos;
- grafos de baja y alta conectividad;
- planificación de una edición pequeña sobre un corpus grande;
- tamaño total de SQLite;
- bytes de índice por documento;
- tamaño máximo del WAL/temporales;
- tiempo de cambio de generación/esquema;
- comportamiento con límites de memoria de contenedor (solo evidencia diagnóstica; no se valida
  cgroup/RSS al abrir).

Separar en las mediciones:

- memoria retenida/controlable contabilizada por Lodestar;
- memoria de SQLite;
- memoria de trabajo y de la cache caliente;
- page cache del sistema operativo;
- memoria mapeada si se utiliza mmap;
- RSS, únicamente como contexto no normativo.

## Conclusión

Lodestar no necesita Redis para resolver el problema fundamental de escala. Ya dispone de la pieza adecuada, SQLite, pero todavía existen rutas runtime que materializan o recorren el workspace completo.

La prioridad debería ser:

1. reutilizar el índice entre arranques;
2. reconstruir en streaming;
3. dejar de requerir `DocumentSet`/`Analysis` globales para operaciones locales;
4. hacer búsqueda, grafo y diagnósticos realmente incrementales;
5. reducir duplicaciones del índice;
6. reemplazar clones globales durante planificación por overlays;
7. introducir Redis únicamente cuando exista una necesidad real de coordinación entre máquinas.

Con este modelo, un corpus de un millón de documentos puede requerir varios GB de almacenamiento persistente —especialmente por el índice textual—, pero la memoria retenida debería depender de
`performance.maxMemory` y no crecer linealmente con todo el corpus. El arranque normal debería
abrir una generación existente y reconciliar cambios, no reconstruir el mundo. Esta política no
resuelve `decisiones §14`, no conecta SQLite y no implementa W-TinyLFU.
