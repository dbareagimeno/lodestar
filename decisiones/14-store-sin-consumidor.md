---
id: 14
titulo: "El store (épica E18) no tiene ningún consumidor"
estado: "abierta"
prioridad: 5
etiquetas: ["store", "rendimiento", "watcher", "descubrimiento"]
origen: "revision-de-pr"
abierta_en: "2026-07-25"
revisada_en: "2026-08-01"
epica: "E23"
historias: ["E23-H16"]
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
