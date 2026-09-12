---
id: 14
titulo: "El store (épica E18) no tiene ningún consumidor"
estado: "abierta"
prioridad: 5
etiquetas: ["store", "rendimiento", "watcher", "descubrimiento"]
origen: "revision-de-pr"
abierta_en: "2026-07-25"
revisada_en: "2026-08-24"
epica: "E23"
historias: ["E23-H16", "E33-H09"]
bloqueada_por: "evidencia disponible; ratificación normativa del usuario pendiente"
relacionadas: [3, 9, 16]
---

# §14 — El store (épica E18) no tiene ningún consumidor

- **Contexto** (detectado en la revisión de la PR #17, 2026-07-25): la épica **E18 entera** —DDL v2
  `documents`/`metadata`/`links`/`diagnostics`, metadata indexada recursivamente por field path con
  su tipo, FTS sin campos privilegiados, cold rebuild, tests de paridad core↔store— está construida,
  verificada… y **ningún consumidor la usa**.
- **Hallazgo verificado**: el único `enable_cache()` del producto está en
  `crates/lodestar-cli/src/commands.rs` (`lodestar reindex`), y solo la **construye**. `App::open` abre
  el `Workspace` sin cache; ninguna de las 10 tools MCP lee de SQLite. `knowledge_search` resuelve por
  `core::text::loose_text_match` sobre el `DocumentSet` en RAM, y `Workspace::document_set()` llama a
  `discovery::discover` en **cada invocación**: relee y reparsea la base entera desde disco en cada
  llamada. Las mediciones de escala de E14-H05 (~10k documentos) son, por tanto, el rendimiento real
  del producto, no el de la cache.
- **Agravante**: el walker del store (`crates/lodestar-store/src/lib.rs`) construye su **propio**
  `ignore::WalkBuilder` y **no aplica la `DiscoveryPolicy`** de `§20.5`: ni `.lodestarignore`, ni
  `discovery.include`/`exclude`, ni `maxDocumentBytes`, ni el endurecimiento de determinismo
  (`parents(false)`, `git_global(false)`, `git_exclude(false)`). E15-H07 declaró explícitamente que
  «la reconfiguración del watcher a la política nueva es parte de E18»; E18 tiene cuatro historias y
  ninguna es esa. Consecuencia: la **paridad core↔store** —invariante 13 de `REFACTOR_PHASE_2` y
  criterio de aceptación— solo se sostiene en workspaces con política por defecto, que es justo el
  caso que ejercitan los tests de paridad. Hoy es inocua porque nadie lee el store; se convierte en
  un bug real el día que se conecte.
- **Qué decidir**: (a) **conectarlo** — `document_set()` lee de SQLite con invalidación por hash y el
  walker del store se alinea con `DiscoveryPolicy`; es la opción que rentabiliza E18 y arregla el
  reparseo por llamada, y la que más código toca; (b) **acotarlo** — declarar por escrito que la
  cache solo sirve a `reindex` y a consumidores externos, y documentar que el motor lee de disco;
  (c) **retirarla** — borrar `lodestar-store` como se borró `lodestar-vcs` en E15-H01, asumiendo que
  el modelo en RAM basta para el tamaño objetivo.
- **Recomendación**: **(a)**, pero **no dentro de E23**. Conectar el store cambia el camino de lectura
  de las 10 tools y toca el invariante #3 («SQLite es cache derivada… cuando podrían discrepar, gana
  el core»), así que merece su propia épica con puerta de diseño, no un apéndice del cierre de la
  migración. Mientras tanto, lo honesto es que esté **registrado**: hoy no es un bug —nada discrepa
  porque nada lee—, pero sí es la mayor cantidad de capacidad construida sin consumidor del repo, y
  la razón por la que cada llamada MCP reparsea la base completa.
- **No bloquea** el merge de la PR #17: el producto funciona, solo que sin cache.

## Repriorización 2026-08-02 — decidir con datos, no con opinión

- **La ficha estaba mal planteada, no mal priorizada**: pide elegir entre conectar / acotar / retirar
  **sin un solo número**. Sin medir, (a) es un salto de fe que toca el invariante #3 y (c) un borrado
  por miedo.
- **CONDICIÓN DE ENTRADA (decidida 2026-08-02)**: el **gate de rendimiento** de
  [`§9`](09-transversales-diferidas.md) punto 1 (cold-open de ~10k documentos, coste por llamada MCP
  con el reparseo actual, y el mismo con cache). §9-bench sube de prioridad y deja de ser un
  transversal aparcado: es el paso previo obligatorio de esta decisión. El banco de pruebas es útil
  se decida lo que se decida.
- **Orden**: va en la **épica de evidencia** (banco de pruebas + dogfooding), **después** de la épica
  de honestidad de superficie. El dogfooding —usar el motor de verdad sobre `decisiones/` y
  `requirements/`— es la otra mitad del dato: dice si el reparseo por llamada molesta en el uso real
  o solo en un arnés sintético.
- **Absorbe de [`§16`](16-deuda-auditoria-e25-e26.md)**, disuelta el mismo día:
  - **(c)** el **watcher** tampoco corre en el motor headless: sin `enable_cache` no hay nada que
    reconciliar, así que hoy el invariante #5 se sostiene por el **protocolo de escritura**
    (temp+fsync+rename por el único camino), no por el watcher. Cualquier opción que se elija tiene
    que decir qué pasa con él.
  - **(l/E26-H09)** El catálogo y `fields.field_path` ya comparten la forma **anclada** publicada por
    el `walk` del core. C6 verifica la paridad observable y la reclasificación cuando solo cambia el
    conjunto de `other_files`; las deudas abiertas de esta ficha siguen siendo el walker sin
    `DiscoveryPolicy` y el destino del watcher.
- **Sigue siendo prioridad 5** y sigue gobernando a las demás: mientras esté abierta,
  `ARCHITECTURE.md §21.5` prohíbe que la superficie externa prometa la cache o el rendimiento a
  escala.

## Evidencia disponible — E33-H08 (2026-08-22)

La condición de entrada ya tiene un paquete trazable: [`evidencia-14-store-2026-08.md`](../docs/qa/evidencia-14-store-2026-08.md).
Incluye la corrida H04 de las tres variantes en las tres escalas, la calibración wire, el número
frío y el dogfooding de H06, además del inventario actual del walker sin `DiscoveryPolicy` y el papel
posible del watcher. La divergencia de `field_path` quedó resuelta por E35-H02; la evidencia está disponible para
decidir, pero este apunte no decide entre conectar, acotar o retirar.

El `estado: "abierta"` y `prioridad: 5` no cambian. La recomendación histórica de conectar (a)
queda registrada como hipótesis pendiente de ratificación: H08 la contrasta con los datos y deja
explícitos sus costes, pero no la convierte en decisión ni la revoca.

## Extensión ratificada — E33-H09 (2026-08-22)

Antes de la decisión final, el usuario exige que el banco acepte una escala positiva arbitraria y
que se ejecute al menos Realista/100k, midiendo también tamaños y RSS por variante. H09 está
ratificada con una iteración a 100k; 1M queda admitido por parametrización y protegido por preflight,
pero no es ejecución obligatoria. El presupuesto actual de disco (`32 KiB × scale + 256 MiB`) es una
heurística modificable, no un contrato ni un criterio de descarte de SQLite. La optimización de
memoria se decide después de medir.

La extensión ya está ejecutada: el [manifiesto de evidencia de v0.6.2](../docs/qa/corridas/v0.6.2/manifest.json)
identifica el bruto externo de H09 y su [resumen](../docs/qa/e33-h09-realista-100k-2026-08-23.md)
contiene 100.004 documentos, 134.783.275 bytes Markdown, 659.578.880 bytes SQLite, equivalencia
funcional entre las tres variantes y pico RSS por worker aislado. El rebuild SQLite fue de
`1_868_411_002_750 ns` (~31,14 min); los picos RSS fueron 972.439.552 bytes (disco), 985.530.368 bytes
(SQLite) y 839.974.912 bytes (RAM), con baseline de 2.637.824 bytes y delta reconciliable por worker.
Esto completa la evidencia técnica solicitada, pero
`§14` sigue abierta: no escoge conectar, acotar ni retirar, no cambia la prioridad y no usa los
techos H05/10k como umbral para la nueva escala. La corrida y la capacidad permanecen en el repo
como resumen y artefacto externo inventariado para evaluar mejoras futuras de rendimiento.

## Adenda de diseño ratificada — E35-H01 / issue #53 (2026-08-24)

La primera ratificación explícita de la issue #53 —titulada originalmente **E34-H01** y trazada
localmente como **E34-H01 → E35-H01**— fija una política para la memoria retenida y controlable:
`N` es el total de
`performance.maxMemory`, `SQLite = floor(30*N/100)`, `W-TinyLFU = floor(20*N/100)` y
`Work = N - SQLite - W-TinyLFU` es la reserva protegida dentro de `N`, que recibe todo residuo. Las
tres partes agotan `N` exactamente: no hay cuarta reserva, pool sin tope ni tamaño abierto, y las
caches nunca invaden `Work`. La configuración tiene default `256MiB`, mínimo `64MiB`, gramática estricta
`[1-9][0-9]*(MiB|GiB)` y conversión a `u64` con aritmética *checked*. El presupuesto no es RSS, no
se valida contra cgroup/RSS al abrir y no añade códigos MCP: los fallos siguen el camino existente de
`INTERNAL_IO_ERROR` con mensaje accionable. El owner único de `MemoryBudget` es
`lodestar-workspace::Workspace::open`.

Los documentos admitidos por `discovery.maxDocumentBytes` podrán procesarse fuera de cache cuando
sea seguro; si no, deben fallar explícitamente sin *thrashing*. Esa semántica se define ahora, pero
la implementación efectiva corresponde a las issues posteriores **#55, #57, #59 y #62**, según la
historia concreta; E35-H01 no promete ejecutarla. Esta adenda cubre config,
`MemoryBudget`/subpresupuestos, tests, mensajes y documentación; no decide la salida de §14, no
conecta SQLite, no implementa la cache W-TinyLFU y no cambia `estado: "abierta"` ni la prioridad de
la ficha. Los detalles normativos están en [`ARCHITECTURE.md §23`](../ARCHITECTURE.md#23-presupuesto-de-memoria-retenida-e35-h01).

## Adenda de implementación — E35-H02 / issue #54 (2026-08-25)

La historia **E35-H02**, trazada desde el título histórico `[E34-H02]` como **E34-H02 → E35-H02**,
reconstruye el esquema derivado con `USER_VERSION = 6`. `documents` usa `doc_id INTEGER PRIMARY KEY`
y ya no contiene `raw`; metadata, diagnostics y enlaces referencian IDs, `fields` internan una sola
vez los paths anclados y los destinos conocidos de enlaces se unen mediante `target_doc_id`. Un
destino que no se puede materializar conserva `target_path` con ID nulo.

La implementación elige FTS5 contentless (`content=''`, `columnsize=0`) según el spike medido y
documentado en [`docs/qa/e35-h02-fts-spike-2026-08-25.md`](../docs/qa/e35-h02-fts-spike-2026-08-25.md):
`524288 bytes` frente a `651264 bytes` de external-content en el mismo test versionado de 10.000
documentos, con reducción `126976` por `docsize`. El único escritor inserta manualmente `rowid = doc_id`, y los candidatos se proyectan
con `JOIN documents`; antes de update/delete lee los valores antiguos exactos —incluido
`documents.body`— para el comando FTS5 `delete`. `documents.body` conserva el snapshot Markdown
completo y exacto como única copia completa de contenido SQLite; `DocumentStore`/core leen el snapshot
desde SQLite. El Markdown en disco sigue siendo la fuente canónica, SQLite permanece cache derivada y
no es lectura por defecto. No hay consumidor App/MCP nuevo; el banco expone `sqlite.dbstat` por objeto
y mantiene el objetivo de footprint `≤ 2,5×` como objetivo no bloqueante, con `gate = false`.

Esta mejora de la cache no decide entre **conectarla**, **acotarla** o **retirarla**. El frontmatter
de esta ficha conserva `estado: "abierta"`, `prioridad: 5` y la recomendación histórica pendiente;
E35-H02 no cambia la prioridad, no cierra §14 y no altera el contrato MCP/CLI.

## Adenda de implementación — E35-H03 / issue #55 (2026-08-26)

E35-H03 sustituye el cold rebuild que acumulaba cuerpos por inventario compacto sin lecturas de
payload y una segunda pasada que lee cada candidato una vez. Los UTF-8 válidos se parsean una sola
vez y los inválidos quedan como `other_files`; la promoción `O(log N)` reata enlaces adelantados sin
crear una segunda verdad semántica. Construye una generación `.next` insert-only, verifica integridad y la publica
atómicamente; fingerprints nacidos en discovery de raíz —incluido el destino real de un symlink—,
entradas y directorios se revalidan tras el streaming y antes del swap para abortar ante cambios
entre pasadas sin otro walker; un authorizer SQLite impide deletes lógicos/prepares no
auditados, el índice previo sigue disponible ante pausa o fallo y los escritores quedan serializados
entre procesos mediante lock nativo RAII. El banco separa RSS del rebuild y memoria controlable, con 60 s/512 MiB como objetivos
`gate=false`. Esto mejora una cache derivada, pero no la conecta a App/MCP ni elige entre conectar,
acotar o retirar: esta ficha conserva `estado: "abierta"` y su prioridad.
