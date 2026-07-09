---
name: historia
description: SDD - redacta o refina una historia en requirements/ (formato E<n>-H<nn>, criterios BDD Dado/Cuando/Entonces, delta de contrato YAML) y la presenta para ratificación. Úsalo al arrancar trabajo que cabe en una historia (para features grandes, /planificar); nunca se implementa sin historia ratificada.
argument-hint: <descripción de la necesidad | ID E<n>-H<nn> a refinar>
---

# /historia — spec primero (SDD)

Convierte una necesidad en una historia implementable de `requirements/`. **Este skill no escribe
código ni tests**: su entregable es la spec.

## Pasos

1. Si el argumento es un ID `E<n>-H<nn>`, localiza la historia existente en `requirements/epica-*.md`
   y trata la invocación como refinamiento; si es una descripción, identifica la épica donde encaja
   (o propone una nueva sección al final de la épica más afín).
2. Lanza el agente **historiador** (tipo `historiador`) con:
   - La descripción/ID tal cual la dio el usuario.
   - El recordatorio de sus reglas: formato exacto de `requirements/README.md`, criterios de
     comportamiento en **Dado/Cuando/Entonces mapeados a nombres de test**, campo Pruebas concreto
     (fichero de test, fixtures, sondas diferenciales), sección «Delta de contrato» si toca la
     frontera (`contracts/ipc.yml`/`contracts/mcp.yml`), trazabilidad §10/§12.
3. Revisa su salida: comprueba que cada criterio es binario y verificable, que no cierra decisiones
   de `DECISIONES.md`, y que el delta de contrato (si existe) referencia tipos de `core::types` por
   nombre sin redefinirlos.
4. Presenta al usuario: ID, resumen, criterios, decisiones abiertas que bloquean, y pide
   **ratificación explícita**. No continúes a `/tdd` sin ella.

## Reglas

- Si la necesidad depende de una decisión abierta de `DECISIONES.md`, la historia se queda en
  borrador y la decisión se le plantea al usuario con opciones — nunca se resuelve por inercia.
- Una historia ratificada es inmutable durante su implementación: si `/tdd` o el juez revelan un
  defecto de spec, se vuelve aquí a refinarla (nueva ratificación), no se reinterpreta en caliente.
