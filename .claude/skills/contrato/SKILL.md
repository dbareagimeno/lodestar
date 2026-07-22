---
name: contrato
description: Verifica o restaura la coherencia de la frontera MCP (contracts/mcp.yml vs tools de lodestar-mcp vs core::types). Con --check solo informa (para antes de un PR); sin flag, sincroniza. Úsalo siempre que un cambio toque la frontera.
argument-hint: "[--check]"
---

# /contrato — la frontera cuenta una sola historia

Delega en el agente **guardian-contrato** la verificación (o sincronización) de las tres superficies
de la frontera MCP: `crates/lodestar-core/src/types.rs` (fuente de tipos), tools de
`crates/lodestar-mcp`, y `contracts/mcp.yml` (spec de superficie/semántica). (La UI de escritorio y
su IPC Tauri —`contracts/ipc.yml`, el espejo `types.ts`— se retiraron a la rama
`experimental/ui-desktop`.)

## Pasos

1. Determina el modo: `--check` → solo informe; sin flag → sincronizar.
2. Si `contracts/mcp.yml` **no existe**, el modo es «generar» (bootstrap por extracción del código
   real — jamás inventando entradas).
3. Lanza el agente **guardian-contrato** (tipo `guardian-contrato`) indicando el modo y sus reglas
   de resolución: en tipos gana `core::types` (nombres §4.1 congelados); en superficie gana el
   código real salvo deltas de historia ratificada marcados `estado: pendiente`.
4. Presenta el informe de drift al usuario con severidades (BLOQUEANTE/MENOR). Si el guardián
   detectó drift que exige tocar **lógica** (no solo YAML), eso es un bug: propón abrir historia con
   `/historia`, no lo arregles aquí.

## Recordatorios

- Córrelo en todo PR que toque `core::types` o las tools MCP: es la red que detecta el drift entre
  las tools reales y `contracts/mcp.yml`.
- El YAML nunca redefine tipos (invariante #4): referencia por nombre de `core::types`.
