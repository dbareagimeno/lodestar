---
id: 14
titulo: "El store (épica E18) no tiene ningún consumidor"
estado: "abierta"
prioridad: 5
etiquetas: ["store", "rendimiento", "watcher", "descubrimiento"]
origen: "revision-de-pr"
abierta_en: "2026-07-25"
revisada_en: "2026-08-23"
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
  - **(l/E26-H09)** **divergencia latente core↔store**: el catálogo publica nombres **anclados**
    (`frontmatter.graph.backlinks`) mientras el store indexa `metadata.field_path` con los nombres
    crudos de `walk`. Hoy no hay discrepancia observable porque esa columna no la lee nadie; es la
    segunda cosa que se rompe el día que se conecte, junto con la `DiscoveryPolicy` del walker.
- **Sigue siendo prioridad 5** y sigue gobernando a las demás: mientras esté abierta,
  `ARCHITECTURE.md §21.5` prohíbe que la superficie externa prometa la cache o el rendimiento a
  escala.

## Evidencia disponible — E33-H08 (2026-08-22)

La condición de entrada ya tiene un paquete trazable: [`evidencia-14-store-2026-08.md`](../docs/qa/evidencia-14-store-2026-08.md).
Incluye la corrida H04 de las tres variantes en las tres escalas, la calibración wire, el número
frío y el dogfooding de H06, además del inventario actual del walker sin `DiscoveryPolicy`, la
divergencia de `field_path` y el papel posible del watcher. La evidencia está disponible para
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
