---
name: planificar
description: Diseñar cambios arquitectónicos o features que no caben en una historia y descomponerlos en una épica ordenada con dos ratificaciones. Usar ante nuevas capacidades, cambios de invariantes, decisiones abiertas o trabajo que afecta varias fronteras; no usar para bugs, docs ni historias acotadas.
---

# Planificar

Convertir una necesidad grande en diseño ratificado y después en historias construibles. Mantener
separadas las decisiones de arquitectura y la descomposición.

## Preparar el contexto

1. Leer `AGENTS.md`, las secciones relevantes de `ARCHITECTURE.md`,
   `docs/REFACTOR_PHASE_2.md`, `IMPLEMENTATION_STATUS.md`, `decisiones/README.md` y las decisiones
   afectadas.
2. Comprobar el estado real del código antes de asumir capacidades o deuda.
3. Identificar restricciones ratificadas, decisiones todavía normativas y frontera MCP afectada.

## Fase A: diseño

Delegar a un agente fresco `planificador` la elaboración de:

- problema, objetivos y no objetivos;
- alternativas viables y trade-offs;
- recomendación y consecuencias sobre invariantes;
- delta concreto de arquitectura, contrato y migración;
- riesgos, reversibilidad y preguntas normativas.

Presentar la propuesta al usuario. No redactar la épica ni implementar hasta obtener ratificación
explícita del diseño. Aplicar la adenda ratificada a los documentos de autoridad antes de continuar.

## Fase B: épica

Tras la primera ratificación, pedir al `planificador` una descomposición por valor observable y
dependencias. Pedir después a un `historiador` fresco que redacte la épica y sus historias con:

- IDs estables, orden de construcción y dependencias;
- criterios Dado/Cuando/Entonces binarios;
- prueba propuesta por criterio, negativos y guardas anti-vacuidad;
- delta de contrato/documentación por historia;
- puertas internas y decisiones que aún requieran al usuario.

Presentar la épica completa para una segunda ratificación. Solo tras ella dejarla como fuente de
trabajo para `$ciclo`. No crear rama ni commit.
