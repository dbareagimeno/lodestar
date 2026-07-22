---
name: juzgar
description: Lanza jueces CIEGOS (agentes frescos que solo reciben la spec y el diff, jamás el contexto de la conversación) sobre el trabajo hecho. Por defecto 1 juez; --panel lanza 3 con lentes distintas y sintetiza. Úsalo antes de commitear/mergear una historia.
argument-hint: "[ID E<n>-H<nn>] [--panel] [--rango <a..b>]"
---

# /juzgar — veredicto sin contaminar

Somete el trabajo a jueces cuya opinión no está contaminada: agentes **frescos** que reciben SOLO
la spec y el diff. La garantía de no-contaminación es tu responsabilidad como orquestador.

## Pasos

1. **Prepara el expediente** (y nada más que el expediente):
   - **Spec**: el texto completo de la historia (`requirements/epica-*.md`). Si no hay ID, usa como
     spec los criterios implícitos: Definición de Done de `requirements/README.md` + invariantes de
     `CLAUDE.md`.
   - **Diff**: `git diff main...HEAD` (rama) o `git diff HEAD` (working tree), o el `--rango` dado.
     Si el diff es enorme, pasa la lista de ficheros + instrucción de computarlo, no un resumen tuyo.
2. **Lanza el juez** (tipo `juez-historia`, agente NUEVO — nunca un fork, nunca `SendMessage` a un
   agente previo) con exactamente: spec, diff (o cómo computarlo) y las rutas de los docs de
   autoridad (`CLAUDE.md`, `ARCHITECTURE.md`, `requirements/README.md`, `DECISIONES.md`).

   **PROHIBIDO** incluir en el prompt: por qué se implementó así, resúmenes de la conversación,
   dificultades encontradas, o tu propia opinión del diff. Si el juez conoce la intención, deja de
   ser ciego.
3. **`--panel`**: lanza los jueces `juez-historia` frescos **en paralelo** (3, o 4 si aplica la
   lente D), mismo expediente, cada uno con una lente declarada al inicio de su prompt:
   - **Lente A — corrección**: ¿el diff cumple cada criterio de aceptación, con test que lo demuestre?
   - **Lente B — invariantes y arquitectura**: pureza del core, único escritor, contrato de tipos
     único, `RelPath`, decisiones §10/§12 no relitigadas.
   - **Lente C — paridad y calidad de tests**: ¿la semántica respeta el prototipo (sondas
     diferenciales incluidas)? ¿Los tests morderían si la implementación estuviera mal?
   - **Lente D — fidelidad UX** (condicional y automática, sin flag): solo si el diff toca
     `frontend/` **y** la historia cita artefactos de `design/` en sus Referencias. El juez recibe
     además esos artefactos (flujos/mockups ratificados) y verifica que el diff los implementa
     fielmente: estados cubiertos (vacío/cargando/error/éxito), tokens CSS del prototipo, patrón
     del artefacto sin desviaciones. Sin artefactos ratificados **no hay lente D** — un juez UX
     sin spec visual opinaría desde el gusto.
4. **Sintetiza** (solo con `--panel`): el veredicto agregado es el **peor** de todas las lentes
   (un RECHAZADA de cualquiera rechaza el conjunto). Deduplica hallazgos y presenta la tabla
   veredicto-por-lente + hallazgos por severidad.
5. **Reporta al usuario** el veredicto tal cual (sin suavizarlo) y, si hay BLOQUEANTE/MAYOR,
   propone el siguiente paso: volver a `/tdd` (defecto de implementación) o a `/historia` (defecto
   de spec). **No apliques arreglos sin que el usuario lo pida.**

## Cuándo escalar a --panel

Historias grandes (>300 líneas de diff), cambios que tocan la frontera front↔back o `core::types`,
y cualquier cambio en las superficies de seguridad (`RelPath`, import/zip, vcs). Para el resto, un
juez basta.
