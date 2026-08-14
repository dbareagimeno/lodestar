---
name: revisar
description: Revisar una entrega o diff de Lodestar con agentes frescos y de solo lectura, separados por corrección, arquitectura y calidad de tests, sintetizando por evidencia. Usar antes de cerrar bugfixes e historias y después de cada reparación; no usar como sustituto de los gates.
---

# Revisar

Juzgar el entregable completo sin heredar las explicaciones del implementador.

## Preparar el expediente

Incluir únicamente:

- spec o reproducción completa;
- diff committed contra `develop`, diff staged, diff unstaged y lista/contenido de ficheros nuevos;
- rutas de `AGENTS.md`, `ARCHITECTURE.md`, `requirements/README.md`, decisiones relevantes y
  `contracts/mcp.yml` cuando aplique;
- evidencia cruda de rojo, lock y gates.

No incluir resúmenes de conversación, decisiones de implementación, dificultades ni opinión del
orquestador.

## Elegir lentes

- Bugfix pequeño: delegar en paralelo a `juez_correccion` y `juez_tests` frescos.
- Historia, frontera MCP, dependencias, rutas, seguridad o transacciones: delegar en paralelo a
  `juez_correccion`, `juez_arquitectura` y `juez_tests` frescos.
- Docs o mecánico: revisar directamente el diff y convocar un juez solo si hay semántica o riesgo.

Crear cada agente sin historial conversacional y mantener su sandbox `read-only`. No reutilizar un
juez después de una reparación.

## Sintetizar por evidencia

- Bloquear por cualquier criterio incumplido, invariante roto o fallo reproducible.
- Convertir una sospecha con escenario plausible pero sin reproducción en investigación, no en
  veto automático.
- No hacer bloqueante un hallazgo especulativo por mayoría ni usar el peor voto como agregador.
- Deduplicar sin perder la atribución de cada juez.
- Distinguir hallazgos preexistentes fuera del diff de regresiones introducidas.

Emitir un veredicto `APROBADA`, `APROBADA CON RESERVAS` o `RECHAZADA`, una matriz
criterio-evidencia, hallazgos con fichero/línea y los gates observados. Tras una reparación, repetir
gates y todo el juicio con agentes frescos.
