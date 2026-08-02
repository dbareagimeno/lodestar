---
id: 3
titulo: "Transporte MCP: stdio propio frente a rmcp oficial"
estado: "abierta"
prioridad: 2
etiquetas: ["mcp", "contrato", "dependencias"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-01"
revisada_en: "2026-07-22"
epica: "E7"
relacionadas: [0, 16]
---

# §3 — Transporte MCP: stdio propio vs `rmcp` oficial (E7)

> **Reafinada por §0/§19 (2026-07-22)**: se mantiene **stdio** y se activa **`outputSchema` vía
> `schemars`** (lo exige el contrato de la superficie 13→10, `REFACTOR §13`); **`rmcp` sigue diferido**
> hasta tener un cliente que lo requiera.


- **Estado**: el MCP funciona como servidor **JSON-RPC por stdio** (stdout puro), con 13 tools y
  test golden cross-fachada (salida de cada tool == `Workspace` directo). Falta el transporte oficial
  `rmcp` + `resources` + `outputSchema` (feature `schemars` ya preparada en el core).
- **Qué decidir**: ¿adoptamos `rmcp` ahora (transporte oficial, resources, negociación de capacidades)
  o mantenemos el stdio propio hasta tener un consumidor que exija `rmcp`?
- **Recomendación**: mantener stdio hasta tener un cliente MCP real que lo requiera; el contrato de
  tools ya está congelado, migrar el transporte después es mecánico.
