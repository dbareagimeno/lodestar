---
id: 9
titulo: "Transversales diferidas de producto"
estado: "abierta"
prioridad: 2
etiquetas: ["rendimiento", "seguridad", "distribucion"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-01"
revisada_en: "2026-07-23"
epica: "E8"
relacionadas: [1, 14]
---

# §9 — Transversales diferidas de producto (E8)

Pendientes de priorización (no bloquean el núcleo):
- **Gate de rendimiento (§11)**: bench de cold-open 10k < ~2s y edit→UI < 150 ms como test de CI.
  El motor incremental ya existe (store); falta el arnés de bench con umbrales.
- **Packaging/release CI + updater + firma** (ligado al punto 1): **CI de release ya existe**
  (`release.yml`, tres plataformas, bundles sin firmar); **queda la firma/notarización + updater**.
- **Threat model** documentado (§12 seguridad); las piezas ya están (RelPath anti path/zip-slip,
  FTS5 escapado, git de red confinado al binario, libgit2 local sin hooks).
- ~~Arnés diferencial JS-vs-Rust (E1-H18)~~ — **hecho y luego RETIRADO en `E15-H04`** (el prototipo
  dejó de ser spec con la migración a Markdown universal, `ARCHITECTURE.md §20.13`). Histórico:
  `prototype/harness/` ejecutaba las funciones
  puras del prototipo en node como oráculo y `tests/differential.rs` compara con el core (6 fixtures);
  cazó y cerró 6 divergencias de paridad.
