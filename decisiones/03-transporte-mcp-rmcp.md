---
id: 3
titulo: "Transporte MCP: stdio propio frente a rmcp oficial"
estado: "abierta"
prioridad: 2
etiquetas: ["mcp", "contrato", "dependencias"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-01"
revisada_en: "2026-08-02"
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

## Absorbe §16(d) (2026-08-02, al disolver [`§16`](16-deuda-auditoria-e25-e26.md))

- **Servidor MCP monohilo, sin *timeout* ni cancelación**: el bucle JSON-RPC atiende **una petición a
  la vez** y no hay forma de cancelar ni de acotar en el tiempo una llamada larga (`knowledge_check`
  sobre una base grande, un `change_plan` con selección masiva). Un cliente que se impaciente no
  tiene protocolo para decirlo.
- **Por qué vive aquí**: es diseño de transporte. Escribir cancelación a mano sobre el stdio propio
  para luego migrar a `rmcp` sería trabajo tirado — así que este punto **refuerza** la recomendación
  de arriba en vez de contradecirla: cuando llegue el cliente que fuerce la decisión, traerá también
  el requisito de cancelación.
- **Prioridad: sigue en 2.** Nadie ha reportado una llamada que se eternice, y las cotas de
  paginación de E26-H10 acotaron el peor caso de las tools de lectura.
