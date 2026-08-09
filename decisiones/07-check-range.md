---
id: 7
titulo: "lodestar check --range a..b"
estado: "obsoleta"
prioridad: 1
etiquetas: ["git", "cli"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-01"
cerrada_en: "2026-07-23"
revisada_en: "2026-07-23"
epica: "E15"
historias: ["E9-H02", "E15-H01"]
superada_por: "ARCHITECTURE.md §20.13"
relacionadas: [0, 6]
---

# §7 — `lodestar check --range a..b`

> **Cerrada por §20 (2026-07-23)**: `--staged`/`--rev`/`--range` se retiraron de la superficie en
> E9-H02 quedando diferidos con el crate `vcs` dormido; al borrarse el crate en E15-H01 dejan de
> tener implementación posible. `check` juzga el working tree y nada más. Registro histórico abajo.

- **Estado**: `--range` juzga **la punta** del rango (equivale a `--rev b`).
- **Qué decidir**: ¿basta con la punta o quieres verificar que **cada commit** del rango es conforme
  (útil para bisect/PR gates)? Lo segundo es más caro pero más estricto.
- **Recomendación**: dejar la punta por defecto y añadir `--each` si en algún momento hace falta el
  barrido por-commit.
