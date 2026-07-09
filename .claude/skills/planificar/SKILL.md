---
name: planificar
description: Convierte una spec o diseño mayor en una épica de historias ordenadas por dependencias, con dos puertas de ratificación (diseño y épica). Un nivel por encima de /historia; úsalo cuando el trabajo no cabe en una sola historia (p. ej. una feature de DECISIONES.md).
argument-hint: <DECISIONES §N | ruta de doc | descripción de la feature>
---

# /planificar — de spec a épica ejecutable

Puerta de entrada de las **features grandes**: cierra el diseño y lo descompone en una épica que
luego `/ciclo` consume historia a historia. Delega el trabajo en el agente **planificador**; tú
orquestas las dos puertas.

## Pasos

1. **Reúne la fuente de la spec**: la sección de `DECISIONES.md`, el doc indicado, o la descripción
   del usuario. Sondea el tamaño: si el alcance cabe en UNA historia, redirige a `/historia` y
   termina — no toda necesidad merece una épica.
2. **Fase A — Diseño**: lanza el agente **planificador** (tipo `planificador`) con la fuente.
   Presenta al usuario su propuesta (decisiones + opciones + recomendación; usa AskUserQuestion si
   son pocas y cerradas). **Puerta: ratificación del diseño.** Tras ratificar, el planificador
   escribe la adenda en `ARCHITECTURE.md` y anota `DECISIONES.md`. Si el diseño ya estaba
   ratificado, la puerta se declara superada con la cita y se pasa a B.
3. **Fase B — Descomposición**: el planificador escribe `requirements/epica-NN-<slug>.md`
   (+ mapa de épicas del README + `trazabilidad.md`). Presenta la tabla resumen (ID · título ·
   dependencias · ¿frontera? · ¿bloqueada?) y el orden de construcción. **Puerta: ratificación de
   la épica** — el usuario puede reordenar, cortar o partir historias antes de aprobar.
4. **Cierre**: propone el roadmap — `/ciclo E<n>-H01`, `/ciclo E<n>-H02`… en el orden de
   construcción, señalando las historias `[BLOQUEADA por DECISIONES §N]` que no pueden arrancar.
   **No lances ningún `/ciclo` por tu cuenta.**

## Reglas

- Las dos puertas no se saltan ni se fusionan: diseñar y trocear son ratificaciones distintas
  (un buen diseño puede estar mal descompuesto, y viceversa).
- El planificador no relitiga decisiones zanjadas (§10/§12) ni cierra abiertas de `DECISIONES.md`
  sin el usuario; si lo ves hacerlo en su salida, devuélvesela.
- Pirámide del flujo: `/planificar` (épica) → `/ciclo` (historia) → `/historia`·`/tdd`·`/contrato`·
  `/juzgar` (etapas). Cada nivel tiene su puerta.
