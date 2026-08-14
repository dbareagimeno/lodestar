---
name: contrato
description: Comprueba la coherencia estructural y semántica de la frontera MCP sin promover el código accidental a norma. Úsalo cuando cambien core::types, tools MCP o contracts/mcp.yml.
argument-hint: "[--check]"
---

# /contrato — frontera MCP

1. Ejecuta `scripts/agent-gates.sh contract` para comparar registro, despacho, perfiles, YAML,
   input/output schemas y tests estructurales existentes.
2. Revisa semánticamente errores, efectos, invariantes y compatibilidad con un juez fresco de
   arquitectura.
3. Si existe un delta ratificado, manda la spec. Si código y contrato divergen sin delta, bloquea.
4. No sincronices el YAML extrayendo automáticamente el código: podría consagrar un cambio
   accidental. Cualquier corrección semántica necesita spec o historia ratificada.

`core::types` sigue siendo la única definición de tipos; el contrato referencia nombres y describe
superficie/semántica.
