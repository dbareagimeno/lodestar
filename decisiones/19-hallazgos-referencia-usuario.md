---
id: 19
titulo: "Hallazgos de documentar la referencia de usuario"
estado: "abierta"
prioridad: 4
etiquetas: ["lenguaje-consulta", "contrato", "docs", "mcp"]
origen: "hallazgo-de-implementacion"
abierta_en: "2026-08-02"
revisada_en: "2026-08-02"
epica: "E27"
historias: ["E27-H11"]
relacionadas: [12, 15, 16]
---

# §19 — Hallazgos de documentar la referencia de usuario

Escribir `docs/user/query-language.md`/`safe-changes.md` ejecutando cada afirmación (regla de la
épica: si documentar destapa un defecto, se registra, no se arregla) dejó tres:

- **(a) `has(frontmatter)` nunca casa; `missing(frontmatter)` casa siempre.** `§20.8` lo lista
  («incluido `has(frontmatter)`»), pero sobre la demo devuelve 0 de 10 y su negación 10 de 10,
  mientras `document.has_frontmatter = true` responde bien (7). Causa: `has(x)` resuelve vía
  `core::eval::resolver_campo` → `FieldPath::sin_anclaje()`, que devuelve `None` para el anclaje
  pelado (`crates/lodestar-core/src/types.rs:507`). Contradice `§20.8` (el contrato no lo
  promete). `query-language.md` lo documenta como límite observado y remite a
  `document.has_frontmatter`. Arreglarlo toca `core::eval` → historia propia.
- **(b) Una `policy` parcial se rechaza aunque el contrato declara ambos campos opcionales.**
  `{"policy": {"requireValidResult": false}}` → `INVALID_SCHEMA: … missing field allowWarnings`.
  `PlanPolicy` tiene `Default` pero sus campos no llevan `#[serde(default)]`
  (`crates/lodestar-core/src/plan.rs:271`), mientras `contracts/mcp.yml` y el `inputSchema` los
  declaran opcionales con default. Omitir `policy` entera sí funciona. Emparenta con `[`§15`](15-parametros-no-declarados.md)`
  (validación de argumentos). `safe-changes.md` instruye enviar ambas claves.
- **(c) Imprecisión en `[`§16(a)`](16-deuda-auditoria-e25-e26.md)` caso (3)**: una clave literal `a.b` y una anidada `a:{b:…}` en
  **documentos distintos** producen **dos filas** del catálogo con el mismo `name` (no una), y
  `mode:"field"` resuelve solo la anidada. La frase del contrato («si ambas formas coinciden en un
  mismo documento comparten una entrada») es consistente con lo observado; la del §16(a) no.

