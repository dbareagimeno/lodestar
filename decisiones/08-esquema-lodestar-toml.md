---
id: 8
titulo: "Esquema de lodestar.toml"
estado: "obsoleta"
prioridad: 1
etiquetas: ["configuracion", "descubrimiento"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-01"
cerrada_en: "2026-07-23"
revisada_en: "2026-07-23"
epica: "E15"
historias: ["E15-H08"]
superada_por: "ARCHITECTURE.md §20.5"
relacionadas: [0]
---

# §8 — Esquema de `lodestar.toml`

> **Cerrada por §20 (2026-07-23)**: `lodestar.toml` se **borra** en E15-H08. Su `[identity]` murió
> con git (E15-H01) y su `[gate]` se absorbe en `.lodestar/config.yaml`, el único fichero de
> configuración (`§20.5`). Lo que la pregunta abierta pedía —override de severidad por código y
> exclusión de rutas— **se concede** en el formato nuevo: `discovery.exclude` y la sección
> `validation:` de `§20.9`, que fija la severidad por familia de diagnóstico. Registro histórico:

- **Estado**: soporta `[gate] block_warnings` (strictness) e `[identity] name/email` (override de
  autor/committer). Defaults seguros (solo `Err` bloquea; identidad por defecto).
- **Qué decidir**: ¿quieres más granularidad, p. ej. **override de severidad por código** (subir/bajar
  un `CheckCode` concreto) o listas de exclusión de rutas?
- **Recomendación**: mantener el esquema mínimo actual hasta tener una necesidad real; es aditivo.
