---
id: 3
titulo: "Transporte MCP: rmcp oficial sobre stdio"
estado: "cerrada"
prioridad: 2
etiquetas: ["mcp", "contrato", "dependencias"]
origen: "puerta-de-diseno"
abierta_en: "2026-07-01"
revisada_en: "2026-08-17"
epica: "E34"
cerrada_por: "E34-H01"
antecedente_issue: 38
pr_superada: 39
relacionadas: [0, 16]
---

# §3 — Transporte MCP: `rmcp` oficial sobre stdio (E34)

> **Cerrada por E34-H01 (2026-08-17)**: PR39 queda superada por la arquitectura dual-era de E34.
> Issue38 es el antecedente que motivó revisar la interoperabilidad; no añade una tercera era ni
> cambia los seis invariantes del workspace.


- **Estado ratificado**: `lodestar-mcp` declara `rmcp = 3.1.2`, Tokio y MSRV `1.88`; el resto del
  workspace conserva MSRV `1.80`. El transporte de producto es stdio, con stdout reservado para
  JSON-RPC y logs en stderr.
- **Política dual-era**: `protocol_policy` es la única fuente de fechas y acepta exactamente Modern
  `2026-07-28` y Legacy `2025-11-25`; Modern es `LATEST` explícito. Las requests stateless con
  fechas ajenas se rechazan; `initialize` siempre negocia la baseline Legacy, sin añadir una rama
  para la fecha antigua o futura solicitada. Las dos eras comparten las diez tools y anuncian
  únicamente `tools`.
- **No objetivos**: no se añaden transports de red ni capacidades MCP fuera de `tools`; las
  historias H02–H06 implementan el servicio, transporte y wire sin reabrir esta decisión.
- **E34-H03 ejecutada**: el bucle manual quedó retirado. `SerialExecutor<LodestarMcpService>` es el
  único `ServerHandler` y se sirve mediante `rmcp::transport::stdio()`. rmcp posee el framing y el
  cierre por EOF; stdout contiene sólo mensajes MCP y los logs permanecen en stderr. Las llamadas
  a tools comparten un único turno serial sobre el `App`.
- **E34-H04 ejecutada**: `LodestarMcpServer` separa la negociación Legacy de la validación
  stateless Modern sin duplicar catálogo ni transporte. Modern sirve discovery, list y call con
  metadata por request con versión, capacidades e identidad no vacía, discriminador completo y
  cache privada no reutilizable; rechaza versiones ajenas, metadata inválida, initialize y ping
  antes del dispatcher.
- **E34-H05 ejecutada**: Legacy negocia siempre su baseline para cualquier revisión string,
  mantiene initialize/initialized/ping/list/call y rechaza discovery como método ausente. Sus
  respuestas no filtran resultType ni hints Modern. La reproducción exacta de issue #38 queda
  verde y demuestra por qué la lista ampliada de PR #39 ya no representa la solución.
- **E34-H06 ejecutada**: `LodestarMcpServer` observa el token de cancelación de rmcp mientras una
  request espera el turno serial y la descarta antes de entrar en el servicio. Tras admitirla, el
  `App` termina su llamada bajo el mismo executor: una cancelación tardía no despublica el canónico
  ni invalida el receipt. Los clientes oficiales ejercen discovery Modern e initialize Legacy; el
  arnés raw fija frames, stdout/stderr y EOF en ambos perfiles. E34 queda cerrada.

## Antecedente absorbido: §16(d) (2026-08-02, al disolver [`§16`](16-deuda-auditoria-e25-e26.md))

- **Estado cerrado por H06**: Tokio/rmcp puede recibir varias requests, pero el executor mantiene
  un único turno de acceso a `App`; no hay doble escritor accidental. Cancelar una request en cola
  impide que ejecute, y cancelar después de admitirla no abandona una transacción a medias.
- **Límite vigente**: no se promete timeout de aplicación. Las cotas de paginación siguen acotando
  las lecturas y la cancelación de transporte no puede saltarse la atomicidad del escritor.
