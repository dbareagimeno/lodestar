---
name: contrato
description: Verifica o restaura la coherencia de la frontera front-back (contracts/*.yml vs comandos Tauri vs tools MCP vs core::types vs types.ts). Con --check solo informa (para antes de un PR); sin flag, sincroniza. Úsalo siempre que un cambio toque la frontera.
argument-hint: "[--check]"
---

# /contrato — la frontera cuenta una sola historia

Delega en el agente **guardian-contrato** la verificación (o sincronización) de las cinco
superficies de la frontera: `crates/lodestar-core/src/types.rs` (fuente de tipos),
`frontend/src/lib/ipc/types.ts` (espejo), tabla de comandos de `src-tauri` + eventos, tools de
`crates/lodestar-mcp`, y `contracts/{ipc,mcp}.yml` (spec de superficie/semántica).

## Pasos

1. Determina el modo: `--check` → solo informe; sin flag → sincronizar.
2. Si `contracts/ipc.yml` o `contracts/mcp.yml` **no existen**, el modo es «generar» (bootstrap por
   extracción del código real — jamás inventando entradas).
3. Lanza el agente **guardian-contrato** (tipo `guardian-contrato`) indicando el modo y sus reglas
   de resolución: en tipos gana `core::types` (nombres §4.1 congelados); en superficie gana el
   código real salvo deltas de historia ratificada marcados `estado: pendiente`.
4. Presenta el informe de drift al usuario con severidades (BLOQUEANTE/MENOR). Si el guardián
   detectó drift que exige tocar **lógica** (no solo espejo/YAML), eso es un bug: propón abrir
   historia con `/historia`, no lo arregles aquí.

## Recordatorios

- Mientras `types.ts` sea espejo manual (hasta cerrar E0-H04/E6-H03 con ts-rs), este skill es la
  única red que detecta su deriva: córrelo en todo PR que toque `core::types`.
- El YAML nunca redefine tipos (invariante #4): referencia por nombre de `core::types`.
