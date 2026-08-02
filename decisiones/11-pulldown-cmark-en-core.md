---
id: 11
titulo: "pulldown-cmark como dependencia de lodestar-core"
estado: "tomada"
prioridad: 1
etiquetas: ["dependencias", "core", "enlaces"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-23"
cerrada_en: "2026-07-23"
revisada_en: "2026-07-23"
epica: "E17"
historias: ["E17-H01"]
---

# §11 — `pulldown-cmark` en `lodestar-core` (E17)

- **Contexto**: la migración exige enlaces Markdown **de referencia** (`[t][id]` con su definición
  `[id]: ../p.md` en otro punto del documento) y **offsets fiables** del destino dentro del cuerpo,
  para reescribirlo en `move_document` (`§20.6`, `§20.11`). Hoy el parser son dos regex
  (`crates/lodestar-core/src/model.rs:16-17,257-258`) que solo ven `[texto](href)`.
- **Decidido (2026-07-23, al escribir la épica E17)**: adoptar `pulldown-cmark` como dependencia de
  `lodestar-core`. Es **pura** (sin I/O, sin runtime, sin C), así que no viola el invariante #2 ni el
  job `core-purity` del CI, que prohíbe `tokio`/`rusqlite`/`git2`/`notify`/`tauri`. Aporta
  resolución nativa de referencias, `link_type` (que es exactamente la clasificación de `§20.6`) y
  `OffsetIter`.
- **Por qué queda anotada aquí**: es la **primera dependencia de parsing** que entra en el core, que
  hasta ahora se autoabastecía con regex. Si prefieres no ampliar la superficie de dependencias del
  core, la alternativa es extender la regex — pero no cubre enlaces de referencia sin reimplementar
  buena parte de un parser Markdown, y los offsets serían menos fiables.
- **Reversible**: solo afecta a `crates/lodestar-core/src/links.rs` (E17-H01). Dilo antes de que
  E17 empiece y se replantea.
