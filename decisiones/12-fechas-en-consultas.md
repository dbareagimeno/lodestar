---
id: 12
titulo: "Comparación de fechas en el lenguaje de consulta"
estado: "cerrada"
prioridad: 1
etiquetas: ["lenguaje-consulta", "contrato", "store"]
origen: "hallazgo-de-implementacion"
abierta_en: "2026-07-23"
cerrada_en: "2026-07-25"
revisada_en: "2026-07-25"
epica: "E23"
historias: ["E23-H14"]
relacionadas: [19]
---

# §12 — Comparación de fechas en el lenguaje de consulta (E19)

- **Contexto** (detectado en la fase roja de E16-H01): `REFACTOR_PHASE_2 §Fase 4` exige soportar
  *"fechas interpretadas como valores YAML"*, y `§Fase 5` pide comparaciones tipadas sin coerción
  implícita (`priority >= 2` funciona, `priority >= "high"` es error de tipo). Pero **`serde_yaml`
  0.9.34 no tiene tipo timestamp**: un `2026-07-23` sin comillas se deserializa como `String`.
- **Consecuencia**: hoy `reviewed_at > "2026-01-01"` sería una comparación de **strings**. Para
  fechas ISO-8601 bien formadas el orden lexicográfico coincide con el cronológico, así que
  «funciona» — pero silenciosamente, y deja de funcionar con formatos mixtos (`2026-7-3`), con
  offsets de zona horaria distintos, o al comparar una fecha con un datetime.
- **Qué decidir**: (a) declarar explícitamente que las fechas son strings y su comparación es
  lexicográfica, documentándolo como limitación; (b) introducir un tipo fecha propio en el core que
  reconozca ISO-8601 al indexar (`§20.12` guarda `value_type` en el store, así que hay sitio);
  (c) cambiar de librería YAML por una que tipe timestamps.
- **Recomendación**: **(a) para E19** —es lo barato y cubre el caso real, que son fechas ISO— y
  reevaluar en E20, cuando `metadata_inspect` tenga que **comunicar** el tipo inferido de cada
  propiedad y la ficción de "todo es string" se note. No bloquea: se puede empezar por (a) y migrar
  a (b) sin romper el wire, porque el tipo viaja en `value_type`.
- **Resolución (E23-H14, 2026-07-25): (a)**, declarado por escrito. E19 y E20 cerraron sin tocar
  esta decisión y la limitación **no estaba documentada en ninguna superficie de usuario** — ni en el
  README ni en el contrato—, que era la mitad peor: un motor que presume de *no coercionar tipos*
  tenía una coerción implícita de facto, sin avisar. Ahora está declarada en `contracts/mcp.yml`
  (semántica de `where`) y en el README.
  **Lo que se declara**: no hay tipo fecha. Un `2026-07-23` sin comillas en el frontmatter es un
  **string** para `serde_yaml` 0.9, y las comparaciones de orden entre strings son **lexicográficas**.
  Para fechas ISO-8601 bien formadas y de la misma longitud eso **coincide** con el orden
  cronológico, así que el caso real funciona; deja de funcionar con formatos mixtos (`2026-7-3`),
  con offsets de zona horaria distintos, o al comparar una fecha con un datetime.
  **Migrar a (b)** —tipo fecha propio en el core, reconocido al indexar— sigue siendo posible sin
  romper el wire, porque el tipo viaja en `value_type` (`§20.12`). Se hará si aparece un caso real
  con formatos mixtos, no antes.
